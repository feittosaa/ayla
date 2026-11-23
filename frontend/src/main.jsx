import { StrictMode, lazy, Suspense } from 'react'
import { createRoot } from 'react-dom/client'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import './index.css'
import App from './App.tsx'

// A janela "athena" é a Athena como bloco de notas; as demais são a Ayla.
const AthenaPad = lazy(() => import('./pad/AthenaPad.tsx'))
let label = 'main'
try { label = getCurrentWebviewWindow().label } catch { /* fora do Tauri */ }

createRoot(document.getElementById('root')).render(
  <StrictMode>
    {label === 'athena' ? <Suspense fallback={null}><AthenaPad /></Suspense> : <App />}
  </StrictMode>,
)
