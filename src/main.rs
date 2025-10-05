// proxy to properly show UTF-8, as simple-http-server (https://github.com/TheWaWaR/simple-http-server) does not. (as of Aug 2025)
// also blocks access to specified folders based on config file

use clap::Parser;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Method, Request, Response, Server, StatusCode};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "proxy")]
#[command(about = "A simple HTTP proxy with folder blocking")]
struct Args {
    #[arg(short = 'c', long = "config", help = "Path to config file")]
    config: PathBuf,
}

#[derive(Deserialize, Serialize, Debug)]
struct Config {
    target: String,
    port: u16,
    blocked_folders: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target: "http://localhost:8000".to_string(),
            port: 8080,
            blocked_folders: vec!["private".to_string()],
        }
    }
}

fn load_config(config_path: &PathBuf) -> Result<Config, Box<dyn std::error::Error>> {
    let config_content = std::fs::read_to_string(config_path)?;
    let config: Config = toml::from_str(&config_content)?;
    Ok(config)
}

fn is_path_blocked(path: &str, blocked_folders: &[String]) -> bool {
    let normalized_path = path.trim_start_matches('/');
    
    for blocked_folder in blocked_folders {
        if normalized_path.starts_with(blocked_folder) {
            // ensure it's exactly the folder or a subfolder (not just a prefix)
            let after_folder = &normalized_path[blocked_folder.len()..];
            if after_folder.is_empty() || after_folder.starts_with('/') {
                return true;
            }
        }
    }
    false
}

async fn proxy_handler(
    req: Request<Body>,
    config: Config,
) -> Result<Response<Body>, Infallible> {
    // check if the requested path is blocked
    let path = req.uri().path();
    if is_path_blocked(path, &config.blocked_folders) {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from("Access to this folder is forbidden"))
            .unwrap());
    }

    // we only handle GET and POST
    if req.method() != Method::GET && req.method() != Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::from("method not allowed (we only use get and post.)"))
            .unwrap());
    }

    let client = Client::new();
    let uri = format!("{}{}", config.target, req.uri().path_and_query().map(|x| x.as_str()).unwrap_or("/"));

    // build the forwarded request
    let mut forwarded_req = Request::builder()
        .method(req.method())
        .uri(uri);

    // copy headers
    for (key, value) in req.headers() {
        forwarded_req = forwarded_req.header(key, value);
    }

    let forwarded_req = forwarded_req.body(req.into_body()).unwrap();

    match client.request(forwarded_req).await {
        Ok(mut response) => {
            // modify Content-Type for text/plain responses
            if let Some(content_type) = response.headers().get("content-type") {
                if content_type.to_str().unwrap_or("").starts_with("text/plain") {
                    response.headers_mut().insert(
                        "content-type",
                        "text/plain; charset=utf-8".parse().unwrap(),
                    );
                }
            }
            Ok(response)
        }
        Err(_) => Ok(Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Body::from("Bad Gateway"))
            .unwrap()),
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    
    let config = match load_config(&args.config) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error loading config file '{}': {}", args.config.display(), e);
            eprintln!("Creating example config file at 'example_config.toml'...");
            
            let example_config = Config::default();
            let example_toml = toml::to_string_pretty(&example_config).unwrap();
            if let Err(write_err) = std::fs::write("example_config.toml", example_toml) {
                eprintln!("Failed to create example config: {}", write_err);
            } else {
                println!("Example config created! Edit it and try again.");
            }
            
            std::process::exit(1);
        }
    };
    
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));

    let config_clone = config.clone();
    let make_svc = make_service_fn(move |_conn| {
        let config = config_clone.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let config = config.clone();
                proxy_handler(req, config)
            }))
        }
    });

    let server = Server::bind(&addr).serve(make_svc);

    println!("proxy running at http://localhost:{}, forwarding to {}", config.port, config.target);
    println!("blocked folders: {:?}", config.blocked_folders);
    println!("config loaded from: {}", args.config.display());

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

// We need to derive Clone for Config to use it in the service closure
impl Clone for Config {
    fn clone(&self) -> Self {
        Self {
            target: self.target.clone(),
            port: self.port,
            blocked_folders: self.blocked_folders.clone(),
        }
    }
}