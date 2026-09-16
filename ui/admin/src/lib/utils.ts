import { type ClassValue, clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function formatDisplayPath(path?: string | null) {
  const value = path?.trim()
  if (!value) return ''

  if (value.startsWith('\\\\?\\UNC\\')) {
    return `\\\\${value.slice('\\\\?\\UNC\\'.length)}`
  }
  if (value.startsWith('\\\\?\\')) {
    return value.slice('\\\\?\\'.length)
  }

  return value
}
