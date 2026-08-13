use tracing_subscriber::filter::LevelFilter;

pub fn init() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .with_target(false)
        .without_time()
        .compact()
        .try_init();
}
