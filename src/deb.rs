use log::{debug, error, info};
use std::io::Write;
use std::path::Path;
use std::process::Command;

pub fn detect_architecture() -> String {
    let output = Command::new("dpkg")
        .arg("--print-architecture")
        .output()
        .expect("Failed to detect system architecture");

    if output.status.success() {
        let arch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        info!("Detected system architecture: {}", arch);
        arch
    } else {
        error!("Failed to detect architecture, falling back to 'amd64'");
        "amd64".to_string()
    }
}

pub struct ControlMetadata<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub arch: &'a str,
    pub maintainer: Option<&'a str>,
    pub description: Option<&'a str>,
    pub deps: Option<&'a str>,
    pub build_deps: Option<&'a str>,
}

pub fn write_control(meta: &ControlMetadata, control_dir: &Path) {
    let mut control = format!(
        concat!(
            "Package: {}\n",
            "Version: {}\n",
            "Architecture: {}\n",
            "Maintainer: {}\n",
            "Description: {}\n"
        ),
        meta.name,
        meta.version,
        meta.arch,
        meta.maintainer.unwrap_or("mkdeb <noreply@example.com>"),
        meta.description.unwrap_or("Auto-packaged by mkdeb")
    );

    if let Some(dep_str) = meta.deps {
        control.push_str(&format!("Depends: {}\n", dep_str));
    }
    if let Some(bdep_str) = meta.build_deps {
        control.push_str(&format!("Build-Depends: {}\n", bdep_str));
    }

    let mut file =
        std::fs::File::create(control_dir.join("control")).expect("Failed to open control file");
    writeln!(file, "{}", control).expect("Failed to write control file");
    debug!("control file:\n{}", control);
}

pub fn build_package(destdir: &Path, output_path: &Path) {
    let mut cmd = Command::new("dpkg-deb");
    cmd.arg("--build")
        .arg("--root-owner-group")
        .arg(destdir)
        .arg(output_path);

    debug!("Running: {:?}", cmd);

    let status = cmd.status().expect("Failed to run dpkg-deb");

    if !status.success() {
        error!("dpkg-deb failed");
        panic!("dpkg-deb build failed");
    }
}

pub fn install_package(deb_path: &Path) {
    let mut cmd = Command::new("sudo");
    cmd.arg("dpkg").arg("-i").arg(deb_path);

    debug!("Running: {:?}", cmd);

    let status = cmd.status().expect("Failed to install package");

    if !status.success() {
        error!("dpkg -i failed for {}", deb_path.display());
        panic!("Installation failed");
    }
}

pub fn get_installed_version(pkg_name: &str) -> Option<String> {
    let mut cmd = Command::new("dpkg-query");
    cmd.args(["-W", "-f=${Version}\n", pkg_name]);

    debug!("Running: {:?}", cmd);

    let output = cmd.output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
