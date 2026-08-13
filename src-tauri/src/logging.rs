use tracing_subscriber::filter::LevelFilter;

pub fn init() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .with_target(false)
        .compact()
        .try_init()
}
