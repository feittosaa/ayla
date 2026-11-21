import { useEffect, useState } from 'react';
import { native } from '../services/Desktop';

type Version = { version: string; latest: string | null; url: string | null };

/** Versão instalada e, se houver tag mais nova no GitHub, o aviso de atualização. */
export default function AppVersion() {
  const [info, setInfo] = useState<Version | null>(null);
  useEffect(() => {
    let alive = true;
    const check = () => void native<Version>('app_version').then(value => { if (alive) setInfo(value); }).catch(() => {});
    check();
    const timer = setInterval(check, 60 * 60 * 1000);
    return () => { alive = false; clearInterval(timer); };
  }, []);
  if (!info) return null;
  return <p className="app-version">
    <span>Ayla {info.version}</span>
    {info.latest && info.url
      ? <button type="button" className="link-button" onClick={() => void native('open_web', { url: info.url }).catch(() => {})}>Atualização {info.latest.replace(/^v/, '')} disponível</button>
      : null}
  </p>;
}
