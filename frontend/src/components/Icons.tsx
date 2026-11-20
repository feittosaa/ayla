import type { ReactNode } from 'react';

/** Ícones de traço, herdando a cor do texto. Sempre decorativos: o rótulo fica no botão. */
function Icon({ size = 20, children }: { size?: number; children: ReactNode }) {
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{children}</svg>;
}
type Props = { size?: number };

export const PlusIcon = ({ size }: Props) => <Icon size={size}><path d="M12 5v14M5 12h14" /></Icon>;
export const SidebarIcon = ({ size }: Props) => <Icon size={size}><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16" /></Icon>;
export const ComputerIcon = ({ size }: Props) => <Icon size={size}><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M2 20h20" /></Icon>;
export const CloudIcon = ({ size }: Props) => <Icon size={size}><path d="M17.5 19a4.5 4.5 0 1 0-1.4-8.78A6 6 0 0 0 4.5 12.5 3.5 3.5 0 0 0 6 19z" /></Icon>;
export const PaperclipIcon = ({ size }: Props) => <Icon size={size}><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" /></Icon>;
export const SlidersIcon = ({ size }: Props) => <Icon size={size}><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6" /></Icon>;
export const MicIcon = ({ size }: Props) => <Icon size={size}><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M8.5 21h7" /></Icon>;
export const ArrowUpIcon = ({ size }: Props) => <Icon size={size}><path d="M12 19V5M5 12l7-7 7 7" /></Icon>;
export const CopyIcon = ({ size }: Props) => <Icon size={size}><rect x="9" y="9" width="13" height="13" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></Icon>;
export const ChevronRightIcon = ({ size }: Props) => <Icon size={size}><path d="M9 18l6-6-6-6" /></Icon>;
export const ChevronLeftIcon = ({ size }: Props) => <Icon size={size}><path d="M15 18l-6-6 6-6" /></Icon>;
export const SettingsIcon = ({ size }: Props) => <Icon size={size}><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.6.85 1 1.51 1H21a2 2 0 1 1 0 4h-.09c-.66 0-1.25.4-1.51 1z" /></Icon>;
export const FileIcon = ({ size }: Props) => <Icon size={size}><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" /></Icon>;
export const CloseIcon = ({ size }: Props) => <Icon size={size}><path d="M18 6L6 18M6 6l12 12" /></Icon>;
export const ImageIcon = ({ size }: Props) => <Icon size={size}><rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="8.5" cy="8.5" r="1.5" /><path d="M21 15l-5-5L5 21" /></Icon>;
export const DownloadIcon = ({ size }: Props) => <Icon size={size}><path d="M12 4v11M7 10l5 5 5-5M5 20h14" /></Icon>;
export const PencilIcon = ({ size }: Props) => <Icon size={size}><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" /></Icon>;
export const TrashIcon = ({ size }: Props) => <Icon size={size}><path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6M10 11v6M14 11v6" /></Icon>;
/** Quadrado cheio: parar gravação ou resposta. */
export const StopIcon = ({ size = 16 }: Props) => <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true"><rect x="5" y="5" width="14" height="14" rx="3" fill="currentColor" /></svg>;
