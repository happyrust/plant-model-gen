use aios_database::fast_model::gen_model::transform_rkyv_cache::TransformCacheFileV1;
use std::collections::HashSet;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() >= 3,
        "usage: dump_transform_cache <cache.rkyv> <refno> [refno...]"
    );
    let path = PathBuf::from(&args[1]);
    let wanted = args[2..].iter().map(String::as_str).collect::<HashSet<_>>();
    let bytes = std::fs::read(&path)?;
    let file = rkyv::from_bytes::<TransformCacheFileV1, rkyv::rancor::Error>(&bytes)
        .map_err(|e| anyhow::anyhow!("decode {} failed: {e:?}", path.display()))?;

    for entry in file.entries {
        if wanted.contains(entry.refno.as_str()) {
            println!(
                "refno={} local={:?} world={:?}",
                entry.refno, entry.local, entry.world
            );
        }
    }
    Ok(())
}
