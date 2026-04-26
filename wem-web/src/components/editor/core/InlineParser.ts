/**
 * Inline markdown parser and DOM serializer for the Wem editor.
 *
 * Supported inline formats (stored in block.content as markdown):
 *   **bold**       → <strong>
 *   *italic*       → <em>
 *   `code`         → <code>
 *   $formula$      → <span class="inline-math">
 *   ==highlight==  → <mark>
 *   ++underline++  → <u>
 */

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

// ─── DOM → Markdown ───

/** Map <b>→strong, <i>→em so nested detection is consistent */
function canonicalTag(tag: string): string {
  if (tag === 'b') return 'strong'
  if (tag === 'i') return 'em'
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
      const tag = (child as HTMLElement).tagName.toLowerCase()
      const canonical = canonicalTag(tag)
      const inner = domToMarkdown(child as HTMLElement, canonical)

      if (canonical === parentCanonical && ['strong', 'em', 'u', 'code', 'mark'].includes(canonical)) {
        out += inner
        continue
      }

      switch (tag) {
        case 'strong': case 'b': out += `**${inner}**`; break
        case 'em': case 'i':     out += `*${inner}*`; break
        case 'code':             out += `\`${inner}\``; break
        case 'mark':             out += `==${inner}==`; break
        case 'u':                out += `++${inner}++`; break
        case 'span': {
          const e = child as HTMLElement
          if (e.classList.contains('inline-math') || e.dataset.type === 'inline-math') {
            if (parentCanonical === 'span-math') { out += inner; continue }
            out += `$${inner}$`
          } else {
            out += inner
          }
          break
        }
        case 'br': break
        default: out += inner; break
      }
    }
  }
  return out
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

  // 2. Merge adjacent same-tag siblings
  const tags = ['strong', 'b', 'em', 'i', 'u', 'code', 'mark']
  for (const tag of tags) {
    root.querySelectorAll(tag).forEach((el) => {
      const next = el.nextElementSibling
      if (next && next.tagName.toLowerCase() === tag) {
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

  const canonical = canonicalTag(tagName)

  // Check if selection is already inside the target element → unwrap
  let node: Node | null = range.commonAncestorContainer
  while (node && node !== el) {
    if (node.nodeType === Node.ELEMENT_NODE) {
      const elem = node as HTMLElement
      const nodeCanonical = canonicalTag(elem.tagName.toLowerCase())
      const matchClass = !className || elem.classList.contains(className)
      if (nodeCanonical === canonical && matchClass) {
        const parent = elem.parentNode!
        while (elem.firstChild) parent.insertBefore(elem.firstChild, elem)
        parent.removeChild(elem)
        return
      }
    }
    node = node.parentNode
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

  normalizeInline(el)
}

/** Remove all inline formatting from the selection, leaving plain text */
export function removeAllFormats(el: HTMLElement): void {
  // First pass: execCommand removes standard formats (strong, b, em, i, u, s)
  document.execCommand('removeFormat')

  // Second pass: unwrap remaining custom elements (mark, code, span.inline-math)
  const tags = ['mark', 'code', 'span']
  for (const tag of tags) {
    el.querySelectorAll(tag).forEach((inner) => {
      if (tag === 'span' && !(inner as HTMLElement).classList.contains('inline-math') && !(inner as HTMLElement).dataset.type) return
      const parent = inner.parentNode!
      while (inner.firstChild) parent.insertBefore(inner.firstChild, inner)
      parent.removeChild(inner)
    })
  }
}
