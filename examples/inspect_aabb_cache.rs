use aios_database::fast_model::AabbCacheFileV1;

fn main() -> anyhow::Result<()> {
    let hash = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: inspect_aabb_cache <geo_hash>"))?
        .parse::<u64>()?;
    let bytes = std::fs::read("assets/meshes/aabb_cache.rkyv")?;
    let cache = rkyv::from_bytes::<AabbCacheFileV1, rkyv::rancor::Error>(&bytes)?;
    let entry = cache
        .entries
        .iter()
        .find(|entry| entry.geo_hash == hash)
        .ok_or_else(|| anyhow::anyhow!("geo_hash {hash} not found"))?;
    println!("{hash} mins={:?} maxs={:?}", entry.mins, entry.maxs);
    Ok(())
}
