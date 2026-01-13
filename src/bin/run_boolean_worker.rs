use std::sync::Arc;

use aios_database::fast_model::mesh_generate::run_boolean_worker;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_option_ext = aios_database::options::get_db_option_ext();
    aios_core::initialize_databases(&db_option_ext.inner).await?;

    let args: Vec<String> = std::env::args().collect();
    let batch_size: usize = args
        .iter()
        .position(|x| x == "--batch-size")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    println!("[bool_worker] batch_size={}", batch_size);
    run_boolean_worker(Arc::new(db_option_ext.inner.clone()), batch_size).await?;
    Ok(())
}

