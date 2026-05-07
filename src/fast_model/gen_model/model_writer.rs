use std::time::{Duration, Instant};

use aios_core::geometry::ShapeInstancesData;

#[derive(Debug, Default)]
pub struct DrainOnlyStats {
    pub batches: usize,
    pub instances: usize,
    pub inst_info: usize,
    pub inst_tubi: usize,
    pub geo_keys: usize,
    pub geo_instances: usize,
    pub neg_relations: usize,
    pub ngmr_relations: usize,
    pub elapsed: Duration,
}

impl DrainOnlyStats {
    fn add_batch(&mut self, batch: &ShapeInstancesData) {
        self.batches += 1;
        self.instances += batch.inst_cnt();
        self.inst_info += batch.inst_info_map.len();
        self.inst_tubi += batch.inst_tubi_map.len();
        self.geo_keys += batch.inst_geos_map.len();
        self.geo_instances += batch
            .inst_geos_map
            .values()
            .map(|geos| geos.insts.len())
            .sum::<usize>();
        self.neg_relations += batch
            .neg_relate_map
            .values()
            .map(Vec::len)
            .sum::<usize>();
        self.ngmr_relations += batch
            .ngmr_neg_relate_map
            .values()
            .map(Vec::len)
            .sum::<usize>();
    }

    pub fn print_summary(&self) {
        println!(
            "[model-writer:drain-only] summary: batches={} instances={} inst_info={} inst_tubi={} geo_keys={} geo_instances={} neg_relations={} ngmr_relations={} elapsed_ms={}",
            self.batches,
            self.instances,
            self.inst_info,
            self.inst_tubi,
            self.geo_keys,
            self.geo_instances,
            self.neg_relations,
            self.ngmr_relations,
            self.elapsed.as_millis()
        );
    }
}

pub async fn run_drain_only_sink(
    receiver: flume::Receiver<ShapeInstancesData>,
) -> anyhow::Result<DrainOnlyStats> {
    let started = Instant::now();
    let mut stats = DrainOnlyStats::default();

    while let Ok(batch) = receiver.recv_async().await {
        stats.add_batch(&batch);

        if stats.batches % 100 == 0 {
            println!(
                "[model-writer:drain-only] drained batches={} instances={} geo_instances={} elapsed_ms={}",
                stats.batches,
                stats.instances,
                stats.geo_instances,
                started.elapsed().as_millis()
            );
        }
    }

    stats.elapsed = started.elapsed();
    Ok(stats)
}
