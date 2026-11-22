use std::{
    env, fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    time::Duration,
};

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use somaris::api::{AppState, router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let initial_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter_layer, filter_reload) = tracing_subscriber::reload::Layer::new(initial_filter);
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer())
        .init();

    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.first().map(String::as_str) {
        Some("mcp") => {
            somaris::mcp::run().await?;
            return Ok(());
        }
        Some("healthcheck") => return healthcheck().await,
        Some("auth") if arguments.get(1).is_some_and(|arg| arg == "reset") => {
            reset_file("admin-auth.json")?;
            stop_service_for_restart();
            return Ok(());
        }
        Some("settings") if arguments.get(1).is_some_and(|arg| arg == "reset") => {
            reset_file("settings.json")?;
            stop_service_for_restart();
            return Ok(());
        }
        Some(command) => {
            return Err(format!(
                "Unknown command {command:?}. Supported commands: mcp, healthcheck, auth reset, settings reset."
            )
            .into());
        }
        None => {}
    }

    let address: SocketAddr = env::var("SOMARIS_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()?;
    let state = AppState::from_environment().await?;
    state
        .install_log_level_reloader(move |level: &str| {
            let filter = EnvFilter::try_new(level).map_err(|error| error.to_string())?;
            filter_reload
                .reload(filter)
                .map_err(|error| error.to_string())
        })
        .await;
    state.log_admin_setup_code().await;
    state.spawn_background_tasks();
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Somaris API listening");
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn healthcheck() -> Result<(), Box<dyn std::error::Error>> {
    let listen: SocketAddr = env::var("SOMARIS_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()?;
    let url = healthcheck_url(listen);
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()?
        .get(url)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(format!("Somaris health endpoint returned {}", response.status()).into());
    }
    Ok(())
}

fn healthcheck_url(listen: SocketAddr) -> String {
    let ip = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let address = SocketAddr::new(ip, listen.port());
    format!("http://{address}/api/v1/health")
}

fn data_directory() -> PathBuf {
    env::var_os("SOMARIS_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"))
}

fn reset_file(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = data_directory().join(name);
    match fs::remove_file(&path) {
        Ok(()) => println!("Removed {}.", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("{} is already reset.", path.display());
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn stop_service_for_restart() {
    let process_one_is_somaris = fs::read_link("/proc/1/exe")
        .ok()
        .and_then(|path| path.file_name().map(|name| name == "somaris"))
        .unwrap_or(false);
    if process_one_is_somaris {
        println!("Stopping Somaris so its supervisor can restart it with the reset configuration.");
        #[cfg(unix)]
        unsafe {
            libc::kill(1, libc::SIGTERM);
        }
    } else {
        println!("Restart Somaris to load the reset configuration and generate a new setup code.");
    }
}

#[cfg(test)]
mod tests {
    use super::healthcheck_url;
    use std::net::{Ipv6Addr, SocketAddr};

    #[test]
    fn healthcheck_uses_a_loopback_address_derived_from_listen() {
        assert_eq!(
            healthcheck_url(SocketAddr::from(([0, 0, 0, 0], 8080))),
            "http://127.0.0.1:8080/api/v1/health"
        );
        assert_eq!(
            healthcheck_url(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 8081))),
            "http://[::1]:8081/api/v1/health"
        );
        assert_eq!(
            healthcheck_url(SocketAddr::from(([192, 168, 1, 20], 8082))),
            "http://192.168.1.20:8082/api/v1/health"
        );
    }
}
