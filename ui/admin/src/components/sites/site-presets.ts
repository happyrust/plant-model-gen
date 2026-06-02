import type { CreateManagedSiteRequest } from '@/types/site'

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
          path: 'D:/AVEVA/Projects/E3D2.1/AvevaPlantSample',
          name: 'AvevaPlantSample',
          role: 'design',
          is_primary: true,
          sort_order: 0,
        },
        {
          path: 'D:/AVEVA/Projects/E3D2.1/AvevaCatalogue',
          name: 'AvevaCatalogue',
          role: 'library',
          is_primary: false,
          sort_order: 1,
        },
      ],
      project_name: 'AvevaPlantSample',
      project_path: 'D:/AVEVA/Projects/E3D2.1/AvevaPlantSample',
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
      pipeline_db_mode: 'file',
      runtime_db_mode: 'ws',
      db_port: 18651,
      web_port: 18650,
      bind_host: '127.0.0.1',
      db_user: 'aveva_site_admin',
      db_password: 'CHANGE_ME_StrongPass_18651!',
      auto_deploy: true,
    },
  },
]
