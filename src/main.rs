mod cli;
mod deb;
mod github;

use crate::cli::CliArgs;
use bytesize::ByteSize;
use chrono::Local;
use clap::Parser;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, error, info};
use prettytable::{format, Cell, Row, Table};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    process::{exit, Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::{fs::File, io::AsyncWriteExt, sync::Mutex, task::JoinHandle};

#[derive(Debug, serde::Deserialize)]
struct FilePackages {
    package: HashMap<String, Package>,
}

pub fn load_all_configs(config_dir: &Path) -> Result<Vec<Package>, Box<dyn std::error::Error>> {
    use std::collections::HashSet;

    let mut packages = Vec::new();
    let mut seen = HashSet::new();
    for entry in fs::read_dir(config_dir)? {
        let path = entry?.path();
        if path.extension().map_or(false, |ext| ext == "toml") {
            let content = fs::read_to_string(&path)?;
            let file_config: FilePackages = toml::from_str(&content)?;
            for (name, mut pkg) in file_config.package {
                pkg.name = name.clone();
                if !seen.insert(pkg.name.clone()) {
                    return Err(format!("Duplicate package name: {}", pkg.name).into());
                }
                packages.push(pkg);
            }
        }
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packages)
}

#[derive(Debug, serde::Deserialize)]
pub struct Package {
    #[serde(default)]
    pub name: String,
    pub repo: String,
    pub version: Option<String>,
    pub configure: Option<String>,
    pub build: Option<String>,
    pub install: Option<String>,
    #[serde(default)]
    pub deps: Option<String>,
    #[serde(default)]
    pub build_deps: Option<String>,
    #[serde(default)]
    pub maintainer: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn spawn_log_task<T: std::io::Read + Send + 'static>(
    reader_source: T,
    log_writer: Arc<Mutex<Option<std::fs::File>>>,
    verbose: u8,
    label: &str,
) -> JoinHandle<()> {
    let label = label.to_string();
    tokio::task::spawn_blocking(move || {
        let reader = std::io::BufReader::new(reader_source);
        for line in reader.lines().map_while(Result::ok) {
            if verbose > 0 {
                println!("[{}] {}", label, line);
            }
            let mut guard = log_writer.blocking_lock();
            if let Some(ref mut f) = *guard {
                writeln!(f, "{}", line).ok();
            }
        }
    })
}

async fn run_command(
    cmd_str: &str,
    cwd: &PathBuf,
    verbose: u8,
    destdir: Option<&str>,
    log_file_path: Option<&Path>,
) {
    let interpolated = if let Some(dest) = destdir {
        cmd_str.replace("{destdir}", dest)
    } else {
        cmd_str.to_string()
    };

    let shell_cmd = if verbose > 1 {
        format!("set -xe; {}", interpolated)
    } else {
        format!("set -e; {}", interpolated)
    };

    let mut cmd = Command::new("bash");
    cmd.args(["-c", &shell_cmd])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("Failed to start command");

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");

    let log_file: Option<std::fs::File> = match log_file_path {
        Some(path) => Some(std::fs::File::create(path).unwrap_or_else(|e| {
            eprintln!("Failed to create log file {:?}: {}", path, e);
            std::process::exit(1);
        })),
        None => None,
    };

    let log_writer = Arc::new(Mutex::new(log_file));

    let handle_stdout = spawn_log_task(stdout, log_writer.clone(), verbose, "stdout");
    let handle_stderr = spawn_log_task(stderr, log_writer.clone(), verbose, "stderr");

    let status = child.wait().expect("Failed to wait on child");

    handle_stdout.await.unwrap();
    handle_stderr.await.unwrap();

    if !status.success() {
        eprintln!("Command failed: {:?}", cmd);
        std::process::exit(1);
    }
}

pub async fn download_with_progress(
    url: &str,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "mkdeb")
        .send()
        .await?
        .error_for_status()?;

    let total_size = response.content_length();

    let pb = match total_size {
        Some(size) => {
            let pb = ProgressBar::new(size);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                    .unwrap()
                    .progress_chars("=> "),
            );
            Some(pb)
        }
        None => {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} {msg}")
                    .unwrap(),
            );
            pb.set_message("Downloading bytes...");
            pb.enable_steady_tick(Duration::from_millis(100));
            Some(pb)
        }
    };

    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();

    let mut downloaded = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(pb) = &pb {
            pb.inc(chunk.len() as u64);
            pb.set_message(format!("{} downloaded", ByteSize(downloaded)));
        }
    }

    if let Some(pb) = pb {
        pb.finish_with_message("Download complete");
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();
    let mut builder = env_logger::Builder::from_default_env();
    builder.format_timestamp(None);

    if args.debug {
        builder.filter_level(log::LevelFilter::Debug);
    } else {
        builder.filter_level(log::LevelFilter::Info);
    }

    builder.init();
    println!("{} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    let config_dir = args
        .config
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("mkdeb")
        });
    let packages = load_all_configs(&config_dir).expect("Failed to load package configs");

    let architecture = deb::detect_architecture();

    let selected = if args.all {
        packages.iter().collect::<Vec<_>>()
    } else if let Some(p) = args.packages.as_deref() {
        let names: Vec<&str> = p.split(',').collect();
        packages
            .iter()
            .filter(|pkg| names.contains(&pkg.name.as_str()))
            .collect()
    } else {
        error!("Please specify --all or -p <packages>");
        exit(1);
    };

    if args.list {
        let mut table = Table::new();
        table.set_format(*format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
        table.add_row(Row::new(vec![
            Cell::new("Package").style_spec("Fb"),
            Cell::new("Installed").style_spec("FY"),
            Cell::new("Config Version").style_spec("Fc"),
        ]));

        for pkg in selected {
            let installed = deb::get_installed_version(&pkg.name).unwrap_or_else(|| "none".into());

            let version = github::find_release(&pkg.repo, pkg.version.as_deref())
                .await
                .map(|r| r.version)
                .unwrap_or_else(|| "(not found)".to_string());

            table.add_row(Row::new(vec![
                Cell::new(&pkg.name),
                Cell::new(&installed),
                Cell::new(&version),
            ]));
        }
        table.printstd();
        return Ok(());
    }

    for pkg in selected {
        let release = github::find_release(&pkg.repo, pkg.version.as_deref())
            .await
            .unwrap_or_else(|| {
                eprintln!("Could not find release for {}", pkg.repo);
                std::process::exit(1);
            });
        let version = release.version;
        let repo_url = release.tarball_url;

        if args.install {
            if let Some(installed_ver) = deb::get_installed_version(&pkg.name) {
                if installed_ver == version {
                    info!("{} {} already installed.", pkg.name, version);
                    return Ok(());
                }
            }
        }

        info!(
            "Building {} version {} tag {} url {}",
            pkg.name, version, release.tag, repo_url
        );

        let work_dir = if let Some(ref path) = args.build_root {
            let path = PathBuf::from(path).join(format!("{}-{}", pkg.name, version));
            fs::create_dir_all(&path).expect("Failed to create build root");
            path
        } else {
            tempfile::tempdir().unwrap().into_path()
        };
        let src_tar = work_dir.join("src.tar.gz");

        debug!("Downloading {}", repo_url);
        download_with_progress(&repo_url, &src_tar).await?;

        let tar = fs::File::open(&src_tar).unwrap();
        let gz = flate2::read::GzDecoder::new(tar);
        let mut archive = tar::Archive::new(gz);
        archive.unpack(&work_dir).unwrap();

        let extracted_dir = fs::read_dir(work_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_type().unwrap().is_dir())
            .unwrap()
            .path();

        let destdir = extracted_dir.canonicalize().unwrap().join("pkg");
        debug!("destdir is {:#?}", destdir);

        fs::create_dir_all(destdir.join("DEBIAN")).unwrap();

        deb::write_control(
            &deb::ControlMetadata {
                name: &pkg.name,
                version: &version,
                arch: &architecture,
                maintainer: pkg.maintainer.as_deref(),
                description: pkg.description.as_deref(),
                deps: pkg.deps.as_deref(),
                build_deps: pkg.build_deps.as_deref(),
            },
            &destdir.join("DEBIAN"),
        );

        let log_dir = args
            .log_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./logs"));
        fs::create_dir_all(&log_dir).ok();

        let timestamp = Local::now().format("%Y%m%d-%H%M%S");
        let configure_log = if args.log {
            Some(log_dir.join(format!("{}-configure-{}.log", pkg.name, timestamp)))
        } else {
            None
        };
        let build_log = if args.log {
            Some(log_dir.join(format!("{}-build-{}.log", pkg.name, timestamp)))
        } else {
            None
        };
        let install_log = if args.log {
            Some(log_dir.join(format!("{}-install-{}.log", pkg.name, timestamp)))
        } else {
            None
        };

        if let Some(cfg) = &pkg.configure {
            run_command(
                cfg,
                &extracted_dir,
                args.verbose,
                Some(destdir.to_str().unwrap()),
                configure_log.as_deref(),
            )
            .await;
        }

        if let Some(bld) = &pkg.build {
            run_command(
                bld,
                &extracted_dir,
                args.verbose,
                Some(destdir.to_str().unwrap()),
                build_log.as_deref(),
            )
            .await;
        }

        if let Some(install_cmd) = &pkg.install {
            run_command(
                install_cmd,
                &extracted_dir,
                args.verbose,
                Some(destdir.to_str().unwrap()),
                install_log.as_deref(),
            )
            .await;
        }

        let deb_name = format!("{}-{}.deb", pkg.name, version);
        let output_path = PathBuf::from(&deb_name);
        deb::build_package(&destdir, &output_path);

        if args.install {
            deb::install_package(&output_path);
        }
    }
    Ok(())
}
