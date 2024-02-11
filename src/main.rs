use actix_proxy::{IntoHttpResponse, SendRequestError};
use actix_web::{middleware, web, App, HttpRequest, HttpResponse, HttpServer};
use clap::Parser;
use log::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Parser)]
struct CliArgs {
    /// 1) If given a JSON file, mapping url to:
    ///     1.1) a directory:
    ///         path: str. Which directory to serve at this url. Default to the url itself.
    ///         index: str. Which file to return accessing a directory. Default to `index.html`.
    ///         dir: bool. For a directory without an index file, whether or not to present its
    ///         content. Default to `true`.
    ///     1.2) another url:
    ///         proxy_to: str. Where to forward this request..
    ///                        e.g. `"/api/get": { "proxy_to": "https://httpbin.org/get" }`
    ///                        forward `/api/get?ans=42` to `https://httpbin.org/get?ans=42`.
    /// 2) If given a directory: Serve this directory at `/`.
    /// 3) If left empty:
    ///     3.1) if `./static_host.json` exists, then use it;
    ///     3.2) otherwise, serve `./` at `/`.
    #[arg(verbatim_doc_comment)]
    config: Option<PathBuf>,
    #[arg(long, default_value_t = 8081)]
    port: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum ConfigItem {
    Proxy {
        proxy_to: String,
    },
    Directory {
        path: Option<PathBuf>,
        index: Option<String>,
        dir: Option<bool>,
    },
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(flatten)]
    map: HashMap<String, ConfigItem>,
}

#[derive(Clone, Debug)]
struct Config {
    items: Vec<(String, ConfigItem)>,
}

impl ConfigFile {
    fn from_directory(path: PathBuf) -> Self {
        let mut map = HashMap::new();
        map.insert(
            "/".to_string(),
            ConfigItem::Directory {
                path: Some(path),
                index: None,
                dir: None,
            },
        );
        Self { map: map }
    }
    fn from_config_path(path: Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(match path {
            None => {
                let default_path = PathBuf::from("static_host.json");
                if default_path.exists() {
                    Self::from_config_path(Some(default_path))?
                } else {
                    Self::from_directory(std::env::current_dir().expect("current directory error"))
                }
            }
            Some(path) => {
                if path.is_file() {
                    serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(path)?))?
                } else {
                    Self::from_directory(path)
                }
            }
        })
    }
}

impl Config {
    fn from_config_path(path: Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self::from_config_file(ConfigFile::from_config_path(path)?))
    }
    fn from_config_file(config_file: ConfigFile) -> Self {
        let mut items: Vec<(String, ConfigItem)> = config_file.map.into_iter().collect();
        items.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Self { items: items }
    }
    fn update_app(&self) -> Box<dyn Fn(&mut web::ServiceConfig) + '_> {
        Box::new(move |cfg: &mut web::ServiceConfig| {
            for (url, item) in &self.items {
                match item {
                    ConfigItem::Directory { path, index, dir } => {
                        let path = path
                            .as_ref()
                            .map_or_else(|| (*url).clone(), |p| p.display().to_string());
                        let mut file_service = actix_files::Files::new(&url, path)
                            .redirect_to_slash_directory()
                            .index_file(index.clone().unwrap_or("index.html".to_string()));
                        if dir.unwrap_or(true) {
                            file_service = file_service.show_files_listing().use_hidden_files();
                        }
                        cfg.service(file_service);
                    }
                    ConfigItem::Proxy { .. } => {
                        cfg.service(web::resource(format!("{url}{{suffix:.*}}")).to(proxy));
                    }
                }
            }
        })
    }
    fn get(&self, input: &String) -> Option<(&String, &ConfigItem)> {
        for (url, item) in &self.items {
            if input.starts_with(url) {
                return Some((url, item));
            }
        }
        None
    }
}

struct AppState {
    config: Config,
    client: awc::Client,
}

async fn proxy(
    path: web::Path<(String,)>,
    req: HttpRequest,
    state: web::Data<AppState>,
) -> Result<HttpResponse, SendRequestError> {
    let url = req.path().to_string();
    let (suffix,) = path.into_inner();
    let query = req.query_string();
    let prefix = match state.config.get(&url) {
        Some((_, ConfigItem::Proxy { proxy_to })) => proxy_to,
        _ => panic!("fail to proxy {url}"),
    };
    let real = if query.len() > 0 {
        format!("{}{}?{}", prefix, suffix, query)
    } else {
        format!("{}{}", prefix, suffix)
    };
    info!("proxy: {url} -> {real}");
    state
        .client
        .get(&real)
        .send()
        .await?
        .into_wrapped_http_response()
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = CliArgs::parse();
    info!("args: {args:?}");
    match Config::from_config_path(args.config) {
        Err(e) => {
            error!("error: {e:?}");
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "config error",
            ))
        }
        Ok(config) => {
            info!("config: {config:?}");
            HttpServer::new(move || {
                App::new()
                    .wrap(middleware::Logger::new("%t %s %T %b %a \"%r\""))
                    .app_data(web::Data::new(AppState {
                        config: config.clone(),
                        client: awc::Client::default(),
                    }))
                    .configure(config.update_app())
            })
            .bind(("0.0.0.0", args.port))?
            .run()
            .await
        }
    }
}
