/**
 * Inline markdown parser and DOM serializer for the Wem editor.
 *
 * Supported inline formats (stored in block.content as markdown):
 *   **bold**       → <strong>
 *   *italic*       → <em>
 *   `code`         → <code>
 *   $formula$      → <span class="inline-math">  (KaTeX rendered)
 *   ==highlight==  → <mark>
 *   ++underline++  → <u>
 */

import katex from 'katex'
import 'katex/dist/katex.min.css'

// ─── AST ───

interface InlineNode {
  type: 'text' | 'strong' | 'em' | 'code' | 'math' | 'mark' | 'u'
  text?: string
  children?: InlineNode[]
}

// ─── Markdown → HTML ───

const MARKERS: readonly { open: string; type: InlineNode['type']; literal: boolean }[] = [
  { open: '`',  type: 'code',   literal: true  },
  { open: '$',  type: 'math',   literal: true  },
  { open: '**', type: 'strong', literal: false },
  { open: '*',  type: 'em',     literal: false },
  { open: '==', type: 'mark',   literal: false },
  { open: '++', type: 'u',      literal: false },
]

function parseInline(src: string): InlineNode[] {
  const nodes: InlineNode[] = []
  let i = 0

  while (i < src.length) {
    let matched = false

    for (const { open, type, literal } of MARKERS) {
      if (!src.startsWith(open, i)) continue

      const contentStart = i + open.length
      const closePos = src.indexOf(open, contentStart)
      if (closePos === -1 || closePos === contentStart) continue

      const content = src.slice(contentStart, closePos)
      nodes.push(literal ? { type, text: content } : { type, children: parseInline(content) })
      i = closePos + open.length
      matched = true
      break
    }

    if (!matched) {
      const last = nodes[nodes.length - 1]
      if (last?.type === 'text') {
        last.text! += src[i]
      } else {
        nodes.push({ type: 'text', text: src[i] })
      }
      i++
    }
  }

  return nodes
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

function renderHtml(nodes: InlineNode[]): string {
  let out = ''
  for (const n of nodes) {
    switch (n.type) {
      case 'text':   out += escapeHtml(n.text!); break
      case 'code':   out += `<code>${escapeHtml(n.text!)}</code>`; break
      case 'math':   out += `<span class="inline-math" data-type="inline-math">${escapeHtml(n.text!)}</span>`; break
      case 'strong': out += `<strong>${renderHtml(n.children!)}</strong>`; break
      case 'em':     out += `<em>${renderHtml(n.children!)}</em>`; break
      case 'mark':   out += `<mark>${renderHtml(n.children!)}</mark>`; break
      case 'u':      out += `<u>${renderHtml(n.children!)}</u>`; break
    }
  }
  return out
}

/** Parse inline markdown to HTML string (for setting innerHTML) */
export function inlineMarkdownToHtml(src: string): string {
  if (!src) return ''
  return renderHtml(parseInline(src))
}

// ─── KaTeX rendering ───

/** Render all un-rendered inline-math spans with KaTeX */
export function renderMathInElement(el: HTMLElement): void {
  el.querySelectorAll('.inline-math:not([data-render="true"])').forEach((node) => {
    const span = node as HTMLElement
    const latex = span.textContent || ''
    if (!latex) return
    span.setAttribute('data-content', latex)
    try {
      katex.render(latex, span, { throwOnError: false })
      span.setAttribute('data-render', 'true')
      span.setAttribute('contenteditable', 'false')
    } catch {
      // keep text content as fallback
    }
  })
}

// ─── DOM → Markdown ───

/** Map <b>→strong, <i>→em so nested detection is consistent */
function canonicalTag(tag: string, className?: string): string {
  if (tag === 'b') return 'strong'
  if (tag === 'i') return 'em'
  if (tag === 'span' && className === 'inline-math') return 'inline-math'
  return tag
}

/** Serialize an element's DOM tree back to inline markdown */
export function domToMarkdown(el: HTMLElement, parentCanonical?: string): string {
  // Fast path: plain text only (no inline elements) — avoids DOM walk on every keystroke
  if (el.childElementCount === 0) return el.textContent || ''

  let out = ''
  for (const child of el.childNodes) {
    if (child.nodeType === Node.TEXT_NODE) {
      out += child.textContent || ''
    } else if (child.nodeType === Node.ELEMENT_NODE) {
      const ce = child as HTMLElement
      const tag = ce.tagName.toLowerCase()
      const cls = ce.className || undefined
      const canonical = canonicalTag(tag, cls)

      // inline-math: read LaTeX from data-content, skip DOM walk
      if (canonical === 'inline-math') {
        if (canonical === parentCanonical) continue
        out += `$${ce.getAttribute('data-content') ?? ce.textContent ?? ''}$`
        continue
      }

      const inner = domToMarkdown(ce, canonical)

      if (canonical === parentCanonical && ['strong', 'em', 'u', 'code', 'mark'].includes(canonical)) {
        out += inner
        continue
      }

      switch (canonical) {
        case 'strong': out += `**${inner}**`; break
        case 'em':     out += `*${inner}*`; break
        case 'code':   out += `\`${inner}\``; break
        case 'mark':   out += `==${inner}==`; break
        case 'u':      out += `++${inner}++`; break
        default: out += inner; break
      }
    }
  }
  return out
}

// ─── DOM text range operations ───

/** Remove characters at [start, end) text-offset range, preserving DOM structure */
export function removeTextRange(el: HTMLElement, start: number, end: number): void {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  let current = 0
  const edits: { node: Text; s: number; e: number }[] = []

  let textNode: Text | null
  while ((textNode = walker.nextNode() as Text | null)) {
    const len = textNode.textContent?.length ?? 0
    const nodeEnd = current + len
    if (nodeEnd > start && current < end) {
      edits.push({
        node: textNode,
        s: Math.max(0, start - current),
        e: Math.min(len, end - current),
      })
    }
    current += len
    if (current >= end) break
  }

  for (const { node, s, e } of edits) {
    const t = node.textContent || ''
    node.textContent = t.slice(0, s) + t.slice(e)
  }
  el.normalize()
}

// ─── DOM normalization ───

/**
 * Flatten nested same-type inline elements and merge adjacent siblings.
 * Called after format operations to keep the DOM clean.
 *
 * e.g. <strong><b>x</b></strong> → <strong>x</strong>
 *      <strong>a</strong><strong>b</strong> → <strong>ab</strong>
 */
export function normalizeInline(root: HTMLElement): void {
  // 1. Flatten nested same-canonical-type: <strong><b>x</b></strong> → <strong>x</strong>
  const nested = root.querySelectorAll(
    'strong strong, strong b, b strong, b b, em em, em i, i em, i i, u u, code code, mark mark',
  )
  nested.forEach((inner) => {
    const parent = inner.parentNode!
    while (inner.firstChild) parent.insertBefore(inner.firstChild, inner)
    parent.removeChild(inner)
  })

  // Flatten nested .inline-math
  root.querySelectorAll('.inline-math .inline-math').forEach((inner) => {
    const parent = inner.parentNode!
    while (inner.firstChild) parent.insertBefore(inner.firstChild, inner)
    parent.removeChild(inner)
  })

  // 2. Merge directly-adjacent same-tag siblings (no text node between them)
  const tags = ['strong', 'b', 'em', 'i', 'u', 'code', 'mark']
  for (const tag of tags) {
    root.querySelectorAll(tag).forEach((el) => {
      const next = el.nextSibling
      if (next && next.nodeType === Node.ELEMENT_NODE && (next as HTMLElement).tagName.toLowerCase() === tag) {
        while (next.firstChild) el.appendChild(next.firstChild)
        next.remove()
      }
    })
  }
}

// ─── Inline format toggle helpers ───

/** Wrap/unwrap selection in an inline element (for code, highlight, math) */
export function toggleInlineWrap(el: HTMLElement, tagName: string, className?: string): void {
  const sel = window.getSelection()
  if (!sel || sel.rangeCount === 0) return

  const range = sel.getRangeAt(0)
  if (range.collapsed) return

  const canonical = canonicalTag(tagName, className)

  // Check if selection is inside a same-type wrapper → unwrap (full or partial)
  let wrapperNode: HTMLElement | null = null
  let node: Node | null = range.commonAncestorContainer
  while (node && node !== el) {
    if (node.nodeType === Node.ELEMENT_NODE) {
      const elem = node as HTMLElement
      if (canonicalTag(elem.tagName.toLowerCase(), elem.className || undefined) === canonical) {
        wrapperNode = elem
        break
      }
    }
    node = node.parentNode
  }

  if (wrapperNode) {
    const parent = wrapperNode.parentNode!

    // Extract content after selection end (still inside wrapper)
    const afterRange = document.createRange()
    afterRange.setStart(range.endContainer, range.endOffset)
    afterRange.setEndAfter(wrapperNode.lastChild || wrapperNode)
    const afterFrag = afterRange.extractContents()

    // Extract content before selection start (still inside wrapper)
    const beforeRange = document.createRange()
    beforeRange.setStartBefore(wrapperNode.firstChild || wrapperNode)
    beforeRange.setEnd(range.startContainer, range.startOffset)
    const beforeFrag = beforeRange.extractContents()

    // Build replacement: [wrapped-before] [selected-text] [wrapped-after]
    const frag = document.createDocumentFragment()

    if (beforeFrag.firstChild) {
      const beforeEl = wrapperNode.cloneNode(false) as HTMLElement
      beforeEl.appendChild(beforeFrag)
      frag.appendChild(beforeEl)
    }

    // Selected content (unwrapped)
    while (wrapperNode.firstChild) frag.appendChild(wrapperNode.firstChild)

    if (afterFrag.firstChild) {
      const afterEl = wrapperNode.cloneNode(false) as HTMLElement
      afterEl.appendChild(afterFrag)
      frag.appendChild(afterEl)
    }

    parent.replaceChild(frag, wrapperNode)
    normalizeInline(el)
    return
  }

  // Wrap selection
  const wrapper = document.createElement(tagName)
  if (className) wrapper.className = className
  try {
    range.surroundContents(wrapper)
  } catch {
    const fragment = range.extractContents()
    wrapper.appendChild(fragment)
    range.insertNode(wrapper)
  }

  // Code and math are literal — strip any inner formatting
  if (canonical === 'code' || canonical === 'inline-math') {
    wrapper.querySelectorAll('strong,b,em,i,u,mark,code').forEach((inner) => {
      if (inner === wrapper) return
      const p = inner.parentNode!
      while (inner.firstChild) p.insertBefore(inner.firstChild, inner)
      p.removeChild(inner)
    })
  }

  normalizeInline(el)
}

/** Remove all inline formatting from the selection, leaving plain text */
export function removeAllFormats(el: HTMLElement): void {
  document.execCommand('removeFormat')

  el.querySelectorAll('mark,code,span.inline-math,span[data-type="inline-math"]').forEach((inner) => {
    const parent = inner.parentNode!
    // Restore LaTeX source text for rendered math spans
    const latex = (inner as HTMLElement).getAttribute('data-content')
    if (latex) {
      const textNode = document.createTextNode(latex)
      parent.replaceChild(textNode, inner)
    } else {
      while (inner.firstChild) parent.insertBefore(inner.firstChild, inner)
      parent.removeChild(inner)
    }
  })
}

// ─── Shared DOM utilities ───

/** Re-render a single inline-math element with updated LaTeX source */
export function renderMathSpan(el: HTMLElement, latex: string): void {
  if (!el.parentElement) return
  if (latex) {
    el.setAttribute('data-content', latex)
    try {
      katex.render(latex, el, { throwOnError: false })
      el.setAttribute('data-render', 'true')
    } catch {
      el.textContent = latex
    }
  } else {
    el.parentElement.replaceChild(document.createTextNode(''), el)
  }
}

/** Walk up from element to find the nearest scrollable parent */
export function findScrollParent(el: HTMLElement): HTMLElement | null {
  let parent = el.parentElement
  while (parent) {
    const { overflowY } = getComputedStyle(parent)
    if (overflowY === 'auto' || overflowY === 'scroll') return parent
    parent = parent.parentElement
  }
  return null
}
