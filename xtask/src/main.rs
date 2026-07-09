use anyhow::{bail, Context, Result};
use octocrab::models::repos::{Asset, Release};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use xshell::{cmd, Shell};

#[tokio::main]
async fn main() -> Result<()> {
    let task = env::args().nth(1).unwrap_or_else(|| "help".to_string());
    match task.as_str() {
        "fetch-core" => fetch_core().await?,
        "dev" => dev().await?,
        "gui" => gui().await?,
        "bundle" => bundle().await?,
        _ => print_help(),
    }
    Ok(())
}

fn print_help() {
    println!("Usage: cargo xtask <command>");
    println!("Commands:");
    println!("  fetch-core   Download mihomo kernel to target/<profile>/");
    println!("  dev          fetch-core + cargo run -p luhomo-service");
    println!("  gui          cargo run -p luhomo-gui");
    println!("  bundle       Build release binaries and package them into dist/");
}

async fn fetch_core() -> Result<()> {
    fetch_core_to_dir(&out_dir()?).await
}

async fn fetch_core_to_dir(out_dir: &Path) -> Result<()> {
    let exe_path = out_dir.join("mihomo.exe");

    if exe_path.exists() {
        println!("mihomo.exe already exists at {}", exe_path.display());
        return Ok(());
    }

    let release = octocrab::instance()
        .repos("MetaCubeX", "mihomo")
        .releases()
        .get_latest()
        .await
        .context("failed to fetch mihomo release info")?;

    let asset = pick_asset(&release)?;
    let url = asset.browser_download_url.as_str();
    println!(
        "Downloading {} ({}) from {}",
        asset.name, release.tag_name, url
    );

    let temp_path = out_dir.join("mihomo.tmp");
    let result = download(url, &temp_path)
        .await
        .and_then(|_| extract(&asset.name, &temp_path, &exe_path));
    let _ = fs::remove_file(&temp_path);
    result?;

    println!("mihomo.exe saved to {}", exe_path.display());
    Ok(())
}

async fn dev() -> Result<()> {
    fetch_core().await?;
    let sh = Shell::new().context("failed to create shell")?;
    cmd!(sh, "cargo run -p luhomo-service")
        .run()
        .context("failed to run luhomo-service")?;
    Ok(())
}

async fn gui() -> Result<()> {
    let sh = Shell::new().context("failed to create shell")?;
    cmd!(sh, "cargo run -p luhomo-gui")
        .run()
        .context("failed to run luhomo-gui")?;
    Ok(())
}

async fn bundle() -> Result<()> {
    let sh = Shell::new().context("failed to create shell")?;

    println!("Building release binaries...");
    cmd!(sh, "cargo build --workspace --release").run()?;

    let root = workspace_root();
    let release_dir = root.join("target").join("release");
    let dist_dir = root.join("dist");

    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir).context("clean dist/ failed")?;
    }
    fs::create_dir_all(&dist_dir).context("create dist/ failed")?;

    println!("Downloading mihomo kernel...");
    fetch_core_to_dir(&release_dir).await?;

    let ext = env::consts::EXE_SUFFIX;
    let binaries = ["luhomo-gui", "luhomo-service", "luhomo-cli", "mihomo"];

    for name in binaries {
        let file_name = format!("{}{}", name, ext);
        let src = release_dir.join(&file_name);
        let dst = dist_dir.join(&file_name);
        fs::copy(&src, &dst).with_context(|| format!("copy {} failed", src.display()))?;
    }

    println!("Bundle ready at {}", dist_dir.display());
    Ok(())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask should be in workspace root")
        .to_path_buf()
}

fn out_dir() -> Result<PathBuf> {
    let mut path = env::current_exe().context("current_exe failed")?;
    path.pop(); // target/debug or target/release
    Ok(path)
}

fn pick_asset(release: &Release) -> Result<&Asset> {
    let target = target_triple()?;
    let prefix = match target.as_str() {
        t if t.contains("windows") && t.contains("x86_64") => "mihomo-windows-amd64-",
        t if t.contains("windows") && t.contains("aarch64") => "mihomo-windows-arm64-",
        t if t.contains("darwin") && t.contains("x86_64") => "mihomo-darwin-amd64-",
        t if t.contains("darwin") && t.contains("aarch64") => "mihomo-darwin-arm64-",
        t if t.contains("linux") && t.contains("x86_64") => "mihomo-linux-amd64-",
        t if t.contains("linux") && t.contains("aarch64") => "mihomo-linux-arm64-",
        _ => bail!("unsupported target: {}", target),
    };

    let candidates: Vec<&Asset> = release
        .assets
        .iter()
        .filter(|a| a.name.starts_with(prefix) && !a.name.contains("compatible"))
        .collect();

    if let Some(asset) = candidates.first() {
        return Ok(asset);
    }

    release
        .assets
        .iter()
        .find(|a| a.name.starts_with(prefix))
        .context(format!(
            "no asset found for target {} in release {}",
            target, release.tag_name
        ))
}

fn target_triple() -> Result<String> {
    let arch = env::consts::ARCH;
    let os = env::consts::OS;
    Ok(format!("{}-{}", arch, os))
}

async fn download(url: &str, dest: &Path) -> Result<()> {
    let mut response = reqwest::get(url).await?.error_for_status()?;
    let mut file = File::create(dest).context("create temp file failed")?;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).context("write chunk failed")?;
    }
    Ok(())
}

fn extract(filename: &str, archive: &Path, out: &Path) -> Result<()> {
    fs::create_dir_all(out.parent().context("out path has no parent")?)?;
    if filename.ends_with(".zip") {
        extract_zip(archive, out)
    } else if filename.ends_with(".gz") {
        extract_gz(archive, out)
    } else {
        bail!("unknown archive format: {}", filename)
    }
}

fn extract_zip(archive: &Path, out: &Path) -> Result<()> {
    let file = File::open(archive).context("open temp archive failed")?;
    let mut archive = zip::ZipArchive::new(file).context("invalid zip")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("zip entry read failed")?;
        let name = file.name().to_lowercase();
        if name.contains("mihomo") && (name.ends_with(".exe") || !name.contains('.')) {
            let mut out_file = File::create(out).context("create output file failed")?;
            std::io::copy(&mut file, &mut out_file).context("copy failed")?;
            return Ok(());
        }
    }
    bail!("no mihomo binary found in zip")
}

fn extract_gz(archive: &Path, out: &Path) -> Result<()> {
    let file = File::open(archive).context("open temp archive failed")?;
    let mut decoder = flate2::read::GzDecoder::new(file);
    let mut out_file = File::create(out).context("create output file failed")?;
    std::io::copy(&mut decoder, &mut out_file).context("gunzip copy failed")?;
    Ok(())
}
