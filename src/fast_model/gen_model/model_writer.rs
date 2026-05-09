use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;

use crate::fast_model::pdms_inst::save_instance_data_with_report;
use crate::options::ModelWriterMode;

#[derive(Debug, Default, Clone)]
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
        self.neg_relations += batch.neg_relate_map.values().map(Vec::len).sum::<usize>();
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

#[derive(Debug, Default, Clone)]
pub struct ModelWriteBatchReport {
    pub missing_neg_carriers: Vec<RefnoEnum>,
}

#[derive(Debug, Default, Clone)]
pub struct ModelWriterFinishReport {
    pub writer_name: &'static str,
    pub drain_only_stats: Option<DrainOnlyStats>,
}

#[async_trait::async_trait]
pub trait ModelWriter: Send + Sync {
    fn name(&self) -> &'static str;

    fn writes_to_surreal(&self) -> bool;

    fn runs_downstream_pipeline(&self) -> bool;

    /// Called once before worker tasks start.
    async fn prepare(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// May be called concurrently by multiple base-writer workers.
    async fn write_batch(
        &self,
        batch: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriteBatchReport>;

    /// Called once after all worker tasks finish.
    async fn finish(&self) -> anyhow::Result<ModelWriterFinishReport> {
        Ok(ModelWriterFinishReport {
            writer_name: self.name(),
            drain_only_stats: None,
        })
    }
}

pub struct SurrealModelWriter {
    mesh_aabb_map: Arc<DashMap<String, Aabb>>,
    missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,
}

impl SurrealModelWriter {
    pub fn new(
        mesh_aabb_map: Arc<DashMap<String, Aabb>>,
        missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,
    ) -> Self {
        Self {
            mesh_aabb_map,
            missing_neg_carriers,
        }
    }
}

#[async_trait::async_trait]
impl ModelWriter for SurrealModelWriter {
    fn name(&self) -> &'static str {
        "surreal"
    }

    fn writes_to_surreal(&self) -> bool {
        true
    }

    fn runs_downstream_pipeline(&self) -> bool {
        true
    }

    async fn write_batch(
        &self,
        batch: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriteBatchReport> {
        let save_report = save_instance_data_with_report(
            batch,
            false,
            &HashMap::new(),
            &self.mesh_aabb_map,
            false,
        )
        .await?;
        if !save_report.missing_neg_carriers.is_empty() {
            let mut guard = self
                .missing_neg_carriers
                .lock()
                .map_err(|_| anyhow::anyhow!("missing_neg_carriers mutex poisoned"))?;
            guard.extend(save_report.missing_neg_carriers.iter().copied());
        }
        Ok(ModelWriteBatchReport {
            missing_neg_carriers: save_report.missing_neg_carriers,
        })
    }
}

pub struct DrainOnlyWriter {
    started: Instant,
    stats: Mutex<DrainOnlyStats>,
}

impl DrainOnlyWriter {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            stats: Mutex::new(DrainOnlyStats::default()),
        }
    }
}

impl Default for DrainOnlyWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ModelWriter for DrainOnlyWriter {
    fn name(&self) -> &'static str {
        "drain-only"
    }

    fn writes_to_surreal(&self) -> bool {
        false
    }

    fn runs_downstream_pipeline(&self) -> bool {
        false
    }

    async fn write_batch(
        &self,
        batch: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriteBatchReport> {
        let progress = {
            let mut stats = self
                .stats
                .lock()
                .map_err(|_| anyhow::anyhow!("drain-only stats mutex poisoned"))?;
            stats.add_batch(batch);
            if stats.batches % 100 == 0 {
                Some((stats.batches, stats.instances, stats.geo_instances))
            } else {
                None
            }
        };

        if let Some((batches, instances, geo_instances)) = progress {
            println!(
                "[model-writer:drain-only] drained batches={} instances={} geo_instances={} elapsed_ms={}",
                batches,
                instances,
                geo_instances,
                self.started.elapsed().as_millis()
            );
        }

        Ok(ModelWriteBatchReport::default())
    }

    async fn finish(&self) -> anyhow::Result<ModelWriterFinishReport> {
        let mut stats = self
            .stats
            .lock()
            .map_err(|_| anyhow::anyhow!("drain-only stats mutex poisoned"))?
            .clone();
        stats.elapsed = self.started.elapsed();
        Ok(ModelWriterFinishReport {
            writer_name: self.name(),
            drain_only_stats: Some(stats),
        })
    }
}

pub fn create_model_writer(
    mode: ModelWriterMode,
    mesh_aabb_map: Arc<DashMap<String, Aabb>>,
    missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,
) -> Arc<dyn ModelWriter> {
    match mode {
        ModelWriterMode::Surreal => {
            Arc::new(SurrealModelWriter::new(mesh_aabb_map, missing_neg_carriers))
        }
        ModelWriterMode::DrainOnly => Arc::new(DrainOnlyWriter::new()),
    }
}

pub async fn run_model_writer_sink(
    receiver: flume::Receiver<ShapeInstancesData>,
    writer: Arc<dyn ModelWriter>,
) -> anyhow::Result<ModelWriterFinishReport> {
    writer.prepare().await?;
    while let Ok(batch) = receiver.recv_async().await {
        writer.write_batch(&batch).await?;
    }
    writer.finish().await
}

pub async fn run_drain_only_sink(
    receiver: flume::Receiver<ShapeInstancesData>,
) -> anyhow::Result<DrainOnlyStats> {
    let report = run_model_writer_sink(receiver, Arc::new(DrainOnlyWriter::new())).await?;
    Ok(report.drain_only_stats.unwrap_or_default())
}
