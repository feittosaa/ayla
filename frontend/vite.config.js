import { cpSync } from 'node:fs'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Leitor de PDF (pdf.js): fontes padrão, mapas de caracteres e decodificadores de imagem
// (JBIG2, JPEG 2000) servidos junto com a interface, em /pdfjs/. Copiados do pacote a cada
// execução; `public/pdfjs` não vai para o Git.
for (const dir of ['cmaps', 'standard_fonts', 'wasm']) {
  cpSync(`node_modules/pdfjs-dist/${dir}`, `public/pdfjs/${dir}`, { recursive: true, filter: path => !/quickjs/.test(path) })
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
})
