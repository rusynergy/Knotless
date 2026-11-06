import { useEffect, useRef, useState } from 'react';
import QRCode from 'qrcode';
import jsQR from 'jsqr';
import { Camera, Copy, RefreshCw } from 'lucide-react';
import { parseVless, type Server } from './domain';
import { serverLink } from './sharing';
import { translator, type Language, type MessageKey } from './i18n';

export function QrScanner({ language, onScan, onStop }: { language: Language; onScan: (text: string) => void; onStop: () => void }) {
  const t = translator(language);
  const video = useRef<HTMLVideoElement>(null);
  const callbacks = useRef({ onScan, onStop }); callbacks.current = { onScan, onStop };
  const [error, setError] = useState<MessageKey | null>(null);
  const [ready, setReady] = useState(false);
  const [cameras, setCameras] = useState<MediaDeviceInfo[]>([]);
  const [device, setDevice] = useState('');
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let cancelled = false, stream: MediaStream | undefined, timer: ReturnType<typeof setTimeout> | undefined;
    const element = video.current!;
    const stop = () => { stream?.getTracks().forEach(track => track.stop()); clearTimeout(timer); element.srcObject = null; };
    setError(null); setReady(false);
    const hidden = () => { if (document.hidden) { stop(); callbacks.current.onStop(); } };
    document.addEventListener('visibilitychange', hidden);
    const start = async () => {
      try {
        if (!navigator.mediaDevices?.getUserMedia) { setError('cameraUnavailable'); return; }
        const acquired = await navigator.mediaDevices.getUserMedia({ audio: false, video: device ? { deviceId: { exact: device } } : { facingMode: 'user', width: { ideal: 1280 }, height: { ideal: 720 } } });
        if (cancelled || document.hidden) { acquired.getTracks().forEach(track => track.stop()); return; }
        stream = acquired;
        stream.getVideoTracks().forEach(track => { track.onended = () => { if (!cancelled) { stop(); setReady(false); setError('cameraBusy'); } }; });
        element.srcObject = stream;
        await element.play();
        if (cancelled) return;
        setReady(true);
        navigator.mediaDevices.enumerateDevices().then(devices => { if (!cancelled) setCameras(devices.filter(d => d.kind === 'videoinput')); }).catch(() => {});
        const canvas = document.createElement('canvas');
        const context = canvas.getContext('2d', { willReadFrequently: true });
        if (!context) throw new Error('Canvas unavailable');
        const scan = () => {
          if (cancelled) return;
          if (element.readyState >= 2 && element.videoWidth) {
            const scale = Math.min(1, 960 / element.videoWidth);
            canvas.width = Math.round(element.videoWidth * scale); canvas.height = Math.round(element.videoHeight * scale);
            context.drawImage(element, 0, 0, canvas.width, canvas.height);
            const frame = context.getImageData(0, 0, canvas.width, canvas.height);
            const code = jsQR(frame.data, frame.width, frame.height);
            if (code) {
              const text = code.data.trim();
              try { parseVless(text); } catch { setError('qrInvalid'); timer = setTimeout(scan, 250); return; }
              stop(); callbacks.current.onScan(text); return;
            }
          }
          timer = setTimeout(scan, 180);
        };
        scan();
      } catch (e) {
        stop();
        if (!cancelled) { setReady(false); const name = (e as DOMException).name; setError(name === 'NotAllowedError' || name === 'SecurityError' ? 'cameraDenied' : name === 'NotFoundError' || name === 'OverconstrainedError' ? 'cameraMissing' : 'cameraBusy'); }
      }
    };
    void start();
    return () => { cancelled = true; stop(); document.removeEventListener('visibilitychange', hidden); };
  }, [device, attempt]);
  return <div className="qr-scanner"><video ref={video} muted playsInline autoPlay aria-label={t('cameraPreview')}/><p className="modal-hint">{t(ready ? 'scanHint' : 'cameraStarting')}</p>{cameras.length > 1 && <select aria-label={t('camera')} value={device} onChange={e => setDevice(e.target.value)}><option value="">{t('cameraDefault')}</option>{cameras.map((c, i) => <option key={c.deviceId} value={c.deviceId}>{c.label || `${t('camera')} ${i + 1}`}</option>)}</select>}{error && <p className="validation invalid" role="alert">{t(error)}</p>}<div className="qr-controls">{error && !ready && <button className="secondary" onClick={() => setAttempt(v => v + 1)}><RefreshCw size={15}/>{t('retryCamera')}</button>}<button className="secondary" onClick={onStop}><Camera size={15}/>{t('stopCamera')}</button></div></div>;
}

export function ShareServer({ server, language }: { server: Server; language: Language }) {
  const t = translator(language);
  const [image, setImage] = useState('');
  const [qrError, setQrError] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  let link = ''; try { link = serverLink(server); } catch { /* Display a localized error without exposing configuration. */ }
  useEffect(() => {
    let active = true; setImage(''); setQrError(false);
    if (link) QRCode.toDataURL(link, { errorCorrectionLevel: 'M', margin: 4, width: 560, color: { dark: '#000000', light: '#ffffff' } }).then(url => { if (active) setImage(url); }).catch(() => { if (active) setQrError(true); });
    return () => { active = false; };
  }, [link]);
  if (!link) return <p className="validation invalid" role="alert">{t('shareUnavailable')}</p>;
  const copy = async () => { try { await navigator.clipboard.writeText(link); setCopied(true); setCopyError(false); } catch { setCopyError(true); } };
  return <div className="share-server"><p className="share-name">{server.name}</p>{image && <img className="share-qr" src={image} alt={t('serverQr')}/>}<p className="modal-hint">{t(qrError ? 'qrTooLong' : 'shareHint')}</p><button autoFocus className="share-link" onClick={copy} title={t('copyLink')} aria-label={t('copyLink')}><code>{link}</code><Copy size={18}/></button><p className={`validation ${copyError ? 'invalid' : ''}`} role="status">{copyError ? t('copyFailed') : copied ? t('copied') : t('clickToCopy')}</p></div>;
}
