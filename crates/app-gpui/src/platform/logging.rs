use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

fn open_log(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub fn init(data_dir: &Path) {
    let path = data_dir.join("logs").join("husk-gpui.log");
    let file = path.parent().and_then(|directory| {
        std::fs::create_dir_all(directory).ok()?;
        open_log(&path).ok()
    });
    let builder = tracing_subscriber::fmt().with_ansi(false).with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "warn,husk_gpui=info".into()),
    );
    if let Some(file) = file {
        let _ = builder.with_writer(Mutex::new(file)).try_init();
    } else {
        let _ = builder.try_init();
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut file) = open_log(&path) {
            let _ = writeln!(
                file,
                "{info}\n{}",
                std::backtrace::Backtrace::force_capture()
            );
        }
        previous(info);
    }));
}
