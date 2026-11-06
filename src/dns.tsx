import { useState } from 'react';
import { translator, type Language } from './i18n';
export type DnsSettings={transport:'tcp'|'doh';ipv6:boolean;address:string;dohUrl:string;leakProtection?:boolean};
export const dnsProviders=[
 {id:'quad9',name:'Quad9',address:'9.9.9.9',dohUrl:'https://dns.quad9.net/dns-query'},
 {id:'google',name:'Google',address:'8.8.8.8',dohUrl:'https://dns.google/dns-query'},
 {id:'cloudflare',name:'Cloudflare',address:'1.1.1.1',dohUrl:'https://1.1.1.1/dns-query'},
 {id:'opendns',name:'OpenDNS',address:'208.67.222.222',dohUrl:'https://doh.opendns.com/dns-query'},
 {id:'yandex',name:'Yandex',address:'77.88.8.8',dohUrl:'https://common.dot.dns.yandex.net/dns-query'},
] as const;
export const defaultDns:DnsSettings={transport:'tcp',ipv6:false,address:dnsProviders[0].address,dohUrl:dnsProviders[0].dohUrl,leakProtection:true};
export function validDns(dns:DnsSettings):boolean{
 if(!['tcp','doh'].includes(dns.transport))return false;
 const ipv4=/^(\d{1,3}\.){3}\d{1,3}$/.test(dns.address)&&dns.address.split('.').every(n=>Number(n)<=255&&!/^0\d/.test(n));
 let ipv6=false;try{ipv6=dns.address.includes(':')&&new URL(`http://[${dns.address}]/`).hostname.length>2;}catch{}
 if(!ipv4&&!ipv6)return false;
 if(ipv4){const first=Number(dns.address.split('.')[0]);if(dns.address==='0.0.0.0'||first===127||(first>=224&&first<=239))return false;}
 if(ipv6){const canonical=new URL(`http://[${dns.address}]/`).hostname;if(canonical==='[::]'||canonical==='[::1]'||canonical.startsWith('[ff'))return false;}
 if(dns.transport==='doh')try{const u=new URL(dns.dohUrl);return u.protocol==='https:'&&!!u.hostname&&!u.username&&!u.password&&!u.hash;}catch{return false;}
 return true;
}
export function DnsPanel({value,onSave,language,running}:{value:DnsSettings;onSave:(v:DnsSettings)=>void;language:Language;running:boolean}){
 const t=translator(language);const [draft,setDraft]=useState(value);const [provider,setProvider]=useState(()=>dnsProviders.find(p=>p.address===value.address&&p.dohUrl===value.dohUrl)?.id||'custom');
 const changed=JSON.stringify(draft)!==JSON.stringify(value);
 return <section className="panel"><h2>DNS</h2><p className="panel-description">{t('dnsHint')}</p><div className="setting-row"><label htmlFor="dns-provider">{t('dnsProvider')}</label><select id="dns-provider" value={provider} onChange={e=>{const id=e.target.value;setProvider(id);const preset=dnsProviders.find(p=>p.id===id);if(preset)setDraft(d=>({...d,address:preset.address,dohUrl:preset.dohUrl}));}}>{dnsProviders.map(p=><option key={p.id} value={p.id}>{p.name}</option>)}<option value="custom">{t('dnsCustom')}</option></select></div><div className="setting-row"><label htmlFor="dns-transport">{t('dnsTransport')}</label><select id="dns-transport" value={draft.transport} onChange={e=>setDraft(d=>({...d,transport:e.target.value as DnsSettings['transport']}))}><option value="tcp">DNS · TCP</option><option value="doh">DNS over HTTPS (DoH)</option></select></div><label className="setting-row"><span>{t('dnsIpv6')}</span><input type="checkbox" checked={draft.ipv6} onChange={e=>setDraft(d=>({...d,ipv6:e.target.checked}))}/></label><p className="panel-description">{t('dnsIpv6Hint')}</p><label className="setting-row"><span>{t('dnsLeakProtection')}</span><input type="checkbox" checked={draft.leakProtection!==false} onChange={e=>setDraft(d=>({...d,leakProtection:e.target.checked}))}/></label><p className="panel-description">{t('dnsLeakProtectionHint')}</p>{draft.leakProtection===false&&<p className="validation invalid" role="status">{t('dnsLeakProtectionOff')}</p>}<label className="field-label dns-address">{t('dnsAddress')}<input readOnly={provider!=='custom'} value={draft.address} onChange={e=>setDraft(d=>({...d,address:e.target.value.trim()}))}/></label>{draft.transport==='doh'&&<><label className="field-label">{t('dnsDohUrl')}<input readOnly={provider!=='custom'} value={draft.dohUrl} onChange={e=>setDraft(d=>({...d,dohUrl:e.target.value.trim()}))}/></label><p className="panel-description">{t('dnsDohHint')}</p></>}{!validDns(draft)&&<p className="validation invalid">{t('DNS_INVALID')}</p>}<button className="primary" disabled={!changed||!validDns(draft)} onClick={()=>onSave(draft)}>{t(changed?'save':'saved')}</button>{running&&<p className="panel-description">{t('dnsReconnect')}</p>}</section>;
}
