import { useState } from 'react';
import { translator, type Language } from './i18n';
import { fragmentMask, type FragmentSettings } from './fragment-config';
export { defaultFragment, type FragmentSettings } from './fragment-config';

export function FragmentPanel({ value, language, onSave }: { value: FragmentSettings; language: Language; onSave: (v: FragmentSettings) => void }) {
  const [draft, setDraft] = useState(value);
  const t = translator(language);
  let valid = true;
  try { if (draft.enabled) fragmentMask(draft); } catch { valid = false; }
  return <section className="panel"><h2>{t('fragmentTitle')}</h2>
    <p className="panel-description">{t('fragmentHint')}</p>
    <label className="setting-row"><strong>{t('fragmentEnable')}</strong><input type="checkbox" checked={draft.enabled} onChange={e=>setDraft({...draft,enabled:e.target.checked})}/></label>
    {draft.enabled && <div className="dns-address">
      <label className="field-label">{t('fragPackets')}<select value={draft.packets} onChange={e=>setDraft({...draft,packets:e.target.value as FragmentSettings['packets']})}><option value="tlshello">TLS ClientHello</option><option value="1-3">{t('fragFirst')}</option></select></label>
      <label className="field-label">{t('fragLength')}<input value={draft.length} onChange={e=>setDraft({...draft,length:e.target.value})}/></label>
      <label className="field-label">{t('fragDelay')}<input value={draft.delay} onChange={e=>setDraft({...draft,delay:e.target.value})}/></label>
    </div>}
    {!valid && <p className="validation invalid" role="alert">{t('fragInvalid')}</p>}
    <div className="editor-actions"><button className="primary" disabled={!valid || JSON.stringify(value)===JSON.stringify(draft)} onClick={()=>onSave(draft)}>{t('save')}</button></div>
  </section>;
}
