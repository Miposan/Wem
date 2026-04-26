import path from 'path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('@codemirror/lang-')) return 'codemirror-langs'
          if (id.includes('/node_modules/@codemirror/') || id.includes('/node_modules/@lezer/')) return 'codemirror'
          if (id.includes('/node_modules/katex/')) return 'katex'
        },
      },
    },
  },
})
