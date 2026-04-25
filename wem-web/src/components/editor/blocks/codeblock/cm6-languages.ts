/**
 * CodeMirror 6 语言映射
 *
 * 将 block_type.language 标识符映射到对应的 CM6 LanguageSupport 扩展。
 * 语言标识符采用 GitHub Linguist 常用名称（小写）。
 *
 * 所有语言包静态导入，后续可按需改为动态加载。
 */
import type { Extension } from '@codemirror/state'

import { javascript } from '@codemirror/lang-javascript'
import { python } from '@codemirror/lang-python'
import { rust } from '@codemirror/lang-rust'
import { css } from '@codemirror/lang-css'
import { html } from '@codemirror/lang-html'
import { json } from '@codemirror/lang-json'
import { markdown } from '@codemirror/lang-markdown'
import { java } from '@codemirror/lang-java'
import { cpp } from '@codemirror/lang-cpp'
import { go } from '@codemirror/lang-go'
import { php } from '@codemirror/lang-php'
import { sql } from '@codemirror/lang-sql'
import { xml } from '@codemirror/lang-xml'
import { yaml } from '@codemirror/lang-yaml'

/** 语言扩展工厂 */
const languageMap: Record<string, () => Extension> = {
  javascript: () => javascript({ jsx: true }),
  js: () => javascript({ jsx: true }),
  typescript: () => javascript({ jsx: true, typescript: true }),
  ts: () => javascript({ jsx: true, typescript: true }),
  python: () => python(),
  py: () => python(),
  rust: () => rust(),
  css: () => css(),
  html: () => html(),
  json: () => json(),
  markdown: () => markdown(),
  md: () => markdown(),
  java: () => java(),
  cpp: () => cpp(),
  c: () => cpp(),
  go: () => go(),
  php: () => php(),
  sql: () => sql(),
  xml: () => xml(),
  yaml: () => yaml(),
  yml: () => yaml(),
}

/**
 * 获取语言扩展
 *
 * @param language 块的语言标识符（如 'rust', 'javascript'）
 * @returns 对应的 CM6 LanguageSupport Extension，未知语言返回空数组
 */
export function getLanguageExtension(language: string): Extension {
  const factory = languageMap[language.toLowerCase()]
  return factory ? factory() : []
}

/**
 * 获取语言的显示名称
 */
export function getLanguageDisplayName(language: string): string {
  const lower = language.toLowerCase()
  const nameMap: Record<string, string> = {
    js: 'JavaScript',
    ts: 'TypeScript',
    py: 'Python',
    md: 'Markdown',
    yml: 'YAML',
    cpp: 'C++',
  }
  return nameMap[lower] ?? (lower.charAt(0).toUpperCase() + lower.slice(1))
}

/** 语言选项列表（用于下拉菜单） */
export const LANGUAGE_OPTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: 'text', label: 'Plain Text' },
  { value: 'javascript', label: 'JavaScript' },
  { value: 'typescript', label: 'TypeScript' },
  { value: 'python', label: 'Python' },
  { value: 'rust', label: 'Rust' },
  { value: 'java', label: 'Java' },
  { value: 'cpp', label: 'C++' },
  { value: 'go', label: 'Go' },
  { value: 'css', label: 'CSS' },
  { value: 'html', label: 'HTML' },
  { value: 'json', label: 'JSON' },
  { value: 'yaml', label: 'YAML' },
  { value: 'sql', label: 'SQL' },
  { value: 'markdown', label: 'Markdown' },
  { value: 'php', label: 'PHP' },
  { value: 'xml', label: 'XML' },
]
