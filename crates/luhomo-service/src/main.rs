#[cfg(windows)]
mod service;

#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    service::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("luhomo-service is intended to run as a Windows service.");
    std::process::exit(1);
}
