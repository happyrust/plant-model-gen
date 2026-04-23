export interface ParseDbTypeOption {
  value: string
  label: string
  detail: string
}

export interface ParsePresetOption {
  key: string
  label: string
  detail: string
  parseDbTypes: string[]
  forceRebuildSystemDb: boolean
}

export const DEFAULT_PARSE_DB_TYPES = ['SYST', 'DESI']

export const PARSE_DB_TYPE_OPTIONS: ParseDbTypeOption[] = [
  {
    value: 'SYST',
    label: 'SYST 系统库',
    detail: '项目系统基础数据。已解析站点再次解析时会优先复用，也可以单独开启强制重建。',
  },
  {
    value: 'DESI',
    label: 'DESI 设计库',
    detail: '设计模型数据。配合手动 DB Nums 可只解析目标库。',
  },
  {
    value: 'CATA',
    label: 'CATA 元件库',
    detail: '元件与规格库数据。',
  },
  {
    value: 'DICT',
    label: 'DICT 字典库',
    detail: '字典与属性定义数据。',
  },
  {
    value: 'GLB',
    label: 'GLB 全局库',
    detail: '全局配置类数据。',
  },
  {
    value: 'GLOB',
    label: 'GLOB 全局库',
    detail: '兼容的全局库类型。',
  },
]

export const MODEL_PARSE_DB_TYPES = ['DESI'] as const
export const SYSTEM_PARSE_DB_TYPES = ['SYST', 'CATA', 'DICT', 'GLB', 'GLOB'] as const

export const MODEL_PARSE_DB_TYPE_OPTIONS = PARSE_DB_TYPE_OPTIONS.filter((option) =>
  MODEL_PARSE_DB_TYPES.includes(option.value as typeof MODEL_PARSE_DB_TYPES[number]),
)

export const SYSTEM_PARSE_DB_TYPE_OPTIONS = PARSE_DB_TYPE_OPTIONS.filter((option) =>
  SYSTEM_PARSE_DB_TYPES.includes(option.value as typeof SYSTEM_PARSE_DB_TYPES[number]),
)

export const PARSE_PRESET_OPTIONS: ParsePresetOption[] = [
  {
    key: 'quick_deploy',
    label: '快速部署',
    detail: 'SYST + DESI，适合最小部署验证。',
    parseDbTypes: ['SYST', 'DESI'],
    forceRebuildSystemDb: false,
  },
  {
    key: 'with_dict',
    label: '带字典',
    detail: 'SYST + DESI + DICT，补齐属性定义。',
    parseDbTypes: ['SYST', 'DESI', 'DICT'],
    forceRebuildSystemDb: false,
  },
  {
    key: 'with_catalogue',
    label: '带元件库',
    detail: 'SYST + DESI + CATA，补齐元件规格。',
    parseDbTypes: ['SYST', 'DESI', 'CATA'],
    forceRebuildSystemDb: false,
  },
  {
    key: 'full_system',
    label: '全量系统数据',
    detail: 'SYST + DESI + CATA + DICT + GLB + GLOB，适合完整系统数据准备。',
    parseDbTypes: ['SYST', 'DESI', 'CATA', 'DICT', 'GLB', 'GLOB'],
    forceRebuildSystemDb: false,
  },
].map((preset) => ({
  ...preset,
  parseDbTypes: normalizeParseDbTypes(preset.parseDbTypes),
}))

export const parseDbTypeLabelMap = Object.fromEntries(
  PARSE_DB_TYPE_OPTIONS.map((option) => [option.value, option.label]),
) as Record<string, string>

export function normalizeParseDbTypes(values: string[]): string[] {
  const allowed = new Set(PARSE_DB_TYPE_OPTIONS.map((option) => option.value))
  return [...new Set(values.map((value) => value.trim().toUpperCase()).filter((value) => allowed.has(value)))].sort()
}

export function splitParseDbTypes(values: string[]) {
  const normalized = normalizeParseDbTypes(values)
  return {
    model: normalized.filter((value) => MODEL_PARSE_DB_TYPES.includes(value as typeof MODEL_PARSE_DB_TYPES[number])),
    system: normalized.filter((value) => SYSTEM_PARSE_DB_TYPES.includes(value as typeof SYSTEM_PARSE_DB_TYPES[number])),
  }
}

export function matchParsePreset(values: string[], forceRebuildSystemDb: boolean) {
  const normalized = normalizeParseDbTypes(values)
  return PARSE_PRESET_OPTIONS.find((preset) =>
    preset.forceRebuildSystemDb === forceRebuildSystemDb
    && preset.parseDbTypes.length === normalized.length
    && preset.parseDbTypes.every((value, index) => value === normalized[index])
  ) ?? null
}
