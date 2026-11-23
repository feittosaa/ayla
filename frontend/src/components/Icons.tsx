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
export const ScreenIcon = ({ size }: Props) => <Icon size={size}><path d="M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3" /><circle cx="12" cy="12" r="2.5" /></Icon>;
export const SlidersIcon = ({ size }: Props) => <Icon size={size}><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6" /></Icon>;
export const MicIcon = ({ size }: Props) => <Icon size={size}><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M8.5 21h7" /></Icon>;
export const ExpandIcon = ({ size }: Props) => <Icon size={size}><path d="M5 19 19 5M8 5h11v11" /></Icon>;
export const FullscreenIcon = ({ size }: Props) => <Icon size={size}><path d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5" /></Icon>;
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
export const EyeIcon = ({ size }: Props) => <Icon size={size}><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z" /><circle cx="12" cy="12" r="3" /></Icon>;
export const TrashIcon = ({ size }: Props) => <Icon size={size}><path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6M10 11v6M14 11v6" /></Icon>;
export const DatabaseIcon = ({ size }: Props) => <Icon size={size}><ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v6c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 11v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6" /></Icon>;
export const WarningIcon = ({ size }: Props) => <Icon size={size}><path d="M12 3 2 20h20L12 3z" /><path d="M12 10v4M12 17.5v.01" /></Icon>;
/** Quadrado cheio: parar gravação ou resposta. */
export const StopIcon = ({ size = 16 }: Props) => <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true"><rect x="5" y="5" width="14" height="14" rx="3" fill="currentColor" /></svg>;
export const LockIcon = ({ size }: Props) => <Icon size={size}><rect x="5" y="11" width="14" height="10" rx="2" /><path d="M8 11V7a4 4 0 0 1 8 0v4" /></Icon>;
export const ImportIcon = ({ size }: Props) => <Icon size={size}><path d="M12 3v12M7 10l5 5 5-5M4 17v3h16v-3" /></Icon>;
export const KeyIcon = ({ size }: Props) => <Icon size={size}><circle cx="8" cy="15" r="4" /><path d="M10.8 12.2 20 3M16 7l3 3M14 9l2 2" /></Icon>;
export const UnlockIcon = ({ size }: Props) => <Icon size={size}><rect x="5" y="11" width="14" height="10" rx="2" /><path d="M8 11V7a4 4 0 0 1 7.5-2" /></Icon>;
export const PinIcon = ({ size }: Props) => <Icon size={size}><path d="M12 17v5M9 3h6l-1 6 3 3v2H7v-2l3-3z" /></Icon>;
export const ColumnsIcon = ({ size }: Props) => <Icon size={size}><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M12 4v16" /></Icon>;
export const GitIcon = ({ size }: Props) => <Icon size={size}><circle cx="6" cy="6" r="2.2" /><circle cx="6" cy="18" r="2.2" /><circle cx="18" cy="8" r="2.2" /><path d="M6 8.2v7.6M18 10.2c0 4-4 4.5-10.2 6.6" /></Icon>;
export const RefreshIcon = ({ size }: Props) => <Icon size={size}><path d="M20 11a8 8 0 0 0-14.6-4.5L4 8M4 4v4h4M4 13a8 8 0 0 0 14.6 4.5L20 16M20 20v-4h-4" /></Icon>;
export const VideoIcon = ({ size }: Props) => <Icon size={size}><rect x="2" y="5" width="15" height="14" rx="2" /><path d="M17 10l5-3v10l-5-3z" /></Icon>;
export const HandIcon = ({ size }: Props) => <Icon size={size}><path d="M18 11V6a2 2 0 0 0-4 0v5M14 10V4a2 2 0 0 0-4 0v6M10 10.5V6a2 2 0 0 0-4 0v8" /><path d="M18 8a2 2 0 0 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.9-6-2.4l-3.6-3.6a2 2 0 0 1 2.8-2.8L7 15" /></Icon>;
export const CursorTextIcon = ({ size }: Props) => <Icon size={size}><path d="M9 4h6M9 20h6M12 4v16" /></Icon>;
export const FitWidthIcon = ({ size }: Props) => <Icon size={size}><path d="M3 12h18M7 8l-4 4 4 4M17 8l4 4-4 4" /></Icon>;
export const FitPageIcon = ({ size }: Props) => <Icon size={size}><rect x="6" y="3" width="12" height="18" rx="1.5" /><path d="M12 7v10M10 9l2-2 2 2M10 15l2 2 2-2" /></Icon>;
