import type { CreateManagedSiteRequest } from '@/types/site'

export const AVEVA_PLANT_SAMPLE_APS250160_DB_FILE =
  'D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample\\aps000\\aps250160_0001'
export const AVEVA_PLANT_SAMPLE_ROOT =
  'D:/AVEVA/Projects/E3D2.1/AvevaPlantSample'
export const AVEVA_CATALOGUE_ROOT =
  'D:/AVEVA/Projects/E3D2.1/AvevaCatalogue'

export interface ManagedSiteFormPreset {
  key: string
  label: string
  detail: string
  badge: string
  form: CreateManagedSiteRequest
}

export const MANAGED_SITE_FORM_PRESETS: ManagedSiteFormPreset[] = [
  {
    key: 'aveva-plant-sample',
    label: 'AvevaPlantSample',
    detail: '填入 AvevaPlantSample + AvevaCatalogue 的多工程示例、完整解析类型、模型生成和本机运行端口。',
    badge: '模板案例',
    form: {
      site_name: 'AvevaPlantSample',
      projects: [
        {
          path: AVEVA_PLANT_SAMPLE_ROOT,
          name: 'AvevaPlantSample',
          role: 'design',
          is_primary: true,
          sort_order: 0,
        },
        {
          path: AVEVA_CATALOGUE_ROOT,
          name: 'AvevaCatalogue',
          role: 'library',
          is_primary: false,
          sort_order: 1,
        },
      ],
      project_name: 'AvevaPlantSample',
      project_path: AVEVA_PLANT_SAMPLE_ROOT,
      project_code: 7011,
      manual_db_nums: [],
      parse_db_types: ['SYST', 'DESI', 'CATA', 'DICT', 'GLB', 'GLOB'],
      force_rebuild_system_db: false,
      gen_model: true,
      gen_mesh: true,
      gen_spatial_tree: true,
      apply_boolean_operation: true,
      mesh_tol_ratio: 3.0,
      export_json: false,
      export_parquet: true,
      pipeline_db_mode: 'ws',
      runtime_db_mode: 'ws',
      db_port: 18651,
      web_port: 18650,
      bind_host: '127.0.0.1',
      db_user: 'aveva_site_admin',
      db_password: 'CHANGE_ME_StrongPass_18651!',
      auto_deploy: true,
    },
  },
  {
    key: 'aveva-plant-sample-aps250160',
    label: 'AvevaPlantSample / aps250160_0001',
    detail: `以 ${AVEVA_PLANT_SAMPLE_APS250160_DB_FILE} 为目标，只配置 dbnum=250160 的单 DB 快速创建部署；保存后可直接在 admin 站点列表看到该配置。`,
    badge: '单库部署',
    form: {
      site_name: 'AvevaPlantSample-aps250160_0001',
      projects: [
        {
          path: AVEVA_PLANT_SAMPLE_ROOT,
          name: 'AvevaPlantSample',
          role: 'design',
          is_primary: true,
          sort_order: 0,
        },
      ],
      project_name: 'AvevaPlantSample',
      project_path: AVEVA_PLANT_SAMPLE_ROOT,
      project_code: 7011,
      manual_db_nums: [250160],
      parse_db_types: [],
      force_rebuild_system_db: false,
      auto_parse_related_dbnums: false,
      gen_model: true,
      gen_mesh: true,
      gen_spatial_tree: true,
      apply_boolean_operation: true,
      mesh_tol_ratio: 3.0,
      export_json: false,
      export_parquet: true,
      pipeline_db_mode: 'ws',
      runtime_db_mode: 'ws',
      bind_host: '127.0.0.1',
      db_user: 'siteadmin250160',
      db_password: 'AdminQuickDeploy@250160',
      auto_deploy: true,
    },
  },
]
