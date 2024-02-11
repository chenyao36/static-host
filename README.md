# static-host

Serve multiple directories as a website.

## Installation

```shell
cargo install --git https://github.com/chenyao36/static-host
```

## Configuration

Example:
```json
{
    "/public": { "path": "/mnt/hdd/public" },
    "/private": { "path": "/mnt/hdd/private", "dir": false },
    "/mnt/ssd/public": {},
    "/api/get": { "proxy_to": "https://httpbin.org/get" }
}
```

