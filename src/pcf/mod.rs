//! PCF（Piping Component File）导出。
//!
//! PCF 是管道行业的通用交换格式，纯文本、按行组织，下游应力分析与预制加工软件
//! （CAESAR II、SPOOLGEN 等）拿它当输入。本模块负责把 PDMS/E3D 的管路设计数据
//! 翻成 PCF 文本。
//!
//! ## 组织方式
//!
//! 一条管路（BRAN）在 PCF 里展开成一串元件记录，因此这里按**元件类型一个文件**拆分：
//! 每个子模块只管生成自己那种元件的字段块，返回 `Vec<u8>` 文本片段，由
//! [`pcf_api::gen_node_basic_data`] 按元件类型分发、再拼成整篇。
//!
//! 元件类型对照（PDMS 类型名 → PCF 记录）：
//! ELBO 弯头、BEND 弯管、TEE 三通、OLET 支管台、REDU 异径管、FLAN 法兰、
//! GASK 垫片、VALV 阀门、INST 仪表、NOZZ 管嘴、CAP 管帽、COUP 管接头、
//! ATTA 附件、TUBI 直管（由前后元件的端点推算，不是独立设计元素）。
//!
//! `bran` 与 `pcf_api` 是公共层：前者提供各元件共用的字段生成原语，
//! 后者提供分发入口与跨元件的属性查询（SKEY、材料码、焊接规格等）。

use aios_core::pdms_types::RefU64;

// BRAN 是管路遍历的入口，也向各元件子模块提供公共字段原语，所以对外可见。
pub mod bran;
mod atta;
mod gask;
mod tubi;
mod elbo;
mod flan;
mod valv;
mod tee;
mod redu;
mod inst;
mod olet;
mod bend;
mod nozz;
mod cap;
mod coup;
mod pcf_api;
// Excel 导出走另一条出口：同一批管道数据换成表格交付。
pub mod excel_api;
