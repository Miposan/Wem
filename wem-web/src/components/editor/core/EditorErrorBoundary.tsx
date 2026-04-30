import { Component, type ReactNode } from 'react'

interface Props {
  children: ReactNode
  blockId?: string
}

interface State {
  hasError: boolean
  prevBlockId?: string
}

export class EditorErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false }

  static getDerivedStateFromError(): State {
    return { hasError: true }
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    if (state.hasError && state.prevBlockId !== props.blockId) {
      return { hasError: false, prevBlockId: props.blockId }
    }
    if (!state.hasError && state.prevBlockId !== props.blockId) {
      return { prevBlockId: props.blockId }
    }
    return null
  }

  componentDidCatch(error: unknown, info: React.ErrorInfo) {
    console.error('[EditorErrorBoundary]', this.props.blockId ?? 'root', error, info.componentStack)
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="wem-error-boundary" style={{ padding: '8px 12px', color: 'var(--color-text-3, #999)', fontSize: '0.85em', fontStyle: 'italic' }}>
          此块渲染出错
        </div>
      )
    }
    return this.props.children
  }
}
