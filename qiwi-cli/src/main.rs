use clap::*;
use phonenumber::PhoneNumber;
use qiwi::*;
use serde::*;
use std::path::*;
use tokio_stream::*;
use tracing_subscriber::{prelude::*, EnvFilter};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Config {
    phone: String,
    token: String,
}

fn config_location() -> PathBuf {
    let mut path = xdg::BaseDirectories::new().unwrap().get_config_home();
    path.push("qiwi-cli/config.toml");

    path
}

#[derive(Debug, Parser)]
enum UnauthorizedCmd {
    /// Authorize client
    Login,
}

#[derive(Debug, Parser)]
#[allow(clippy::large_enum_variant)]
enum AuthorizedCmd {
    /// Reauthorize client
    Login,
    /// Get profile info,
    ProfileInfo,
    /// Get payment history,
    PaymentHistory,
    CommissionInfo {
        provider: ProviderId,
    },
}

async fn do_authorize() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut stdin = tokio_util::codec::FramedRead::new(
        tokio::io::stdin(),
        tokio_util::codec::LinesCodec::new(),
    );

    println!("Please enter user ID");

    let phone = stdin
        .next()
        .await
        .unwrap_or_else(|| std::process::exit(0))?
        .parse::<PhoneNumber>()?
        .to_string();

    println!("Please enter your token");

    let token = stdin
        .next()
        .await
        .unwrap_or_else(|| std::process::exit(0))?;

    let path = config_location();
    println!("Saving token on disk to {}", path.to_string_lossy());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    tokio::fs::write(
        path,
        toml::to_string(&Config { phone, token })?.into_bytes(),
    )
    .await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filter = EnvFilter::from_default_env();
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();

    let config = async move {
        if let Ok(data) = tokio::fs::read(config_location()).await {
            if let Ok(config) = toml::from_str::<Config>(&String::from_utf8(data).unwrap()) {
                return Some(config);
            }
        }

        None
    }
    .await;

    match config {
        None => match UnauthorizedCmd::parse() {
            UnauthorizedCmd::Login => do_authorize().await?,
        },
        Some(config) => match AuthorizedCmd::parse() {
            AuthorizedCmd::Login => do_authorize().await?,
            other => {
                println!("Using config {config:?}");
                let client = Client::new(config.phone.parse()?, config.token);
                let _ = client;
                match other {
                    AuthorizedCmd::ProfileInfo => {
                        let profile_info = client.profile_info().await?;
                        println!("Profile info:");
                        println!("{profile_info:?}");
                    }
                    AuthorizedCmd::PaymentHistory => {
                        while let Some(entry) = client.payment_history().next().await.transpose()? {
                            println!("{entry:?}");
                        }
                    }
                    AuthorizedCmd::CommissionInfo { provider } => {
                        println!("{:?}", client.commission_info(provider).await?)
                    }
                    other => unimplemented!("{other:?}"),
                }
            }
        },
    };

    Ok(())
}
