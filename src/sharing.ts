import { parseVless, type Server } from './domain';

// Reconstruct links for existing profiles as well as subscription entries.
export function serverLink(server: Server): string {
  const out = server.outbound as { protocol?: string; settings?: { vnext?: { address: string; port: number; users: { id: string; flow?: string }[] }[] }; streamSettings?: Record<string, any> };
  const endpoint = out.settings?.vnext?.[0];
  const user = endpoint?.users?.[0];
  if (out.protocol !== 'vless' || !endpoint || !user) throw new Error('shareUnavailable');
  const stream = out.streamSettings || {};
  const type = stream.network || 'tcp';
  const security = stream.security || 'none';
  const params = new URLSearchParams({ encryption: 'none', type, security });
  const put = (key: string, value: unknown) => { if (typeof value === 'string' && value) params.set(key, value); };
  put('flow', user.flow);
  const tls = security === 'reality' ? stream.realitySettings : stream.tlsSettings;
  if (tls) {
    put('sni', tls.serverName); put('fp', tls.fingerprint);
    if (Array.isArray(tls.alpn)) put('alpn', tls.alpn.join(','));
    if (security === 'reality') { put('pbk', tls.publicKey); put('sid', tls.shortId); put('spx', tls.spiderX); }
  }
  if (type === 'ws') { put('path', stream.wsSettings?.path); put('host', stream.wsSettings?.headers?.Host); }
  if (type === 'grpc') { put('serviceName', stream.grpcSettings?.serviceName); if (stream.grpcSettings?.multiMode) put('mode', 'multi'); }
  if (type === 'xhttp') { put('path', stream.xhttpSettings?.path); put('host', stream.xhttpSettings?.host); put('mode', stream.xhttpSettings?.mode); }
  if (type === 'httpupgrade') { put('path', stream.httpupgradeSettings?.path); put('host', stream.httpupgradeSettings?.host); }
  const host = endpoint.address.includes(':') ? `[${endpoint.address}]` : endpoint.address;
  const link = `vless://${encodeURIComponent(user.id)}@${host}:${endpoint.port}?${params}#${encodeURIComponent(server.name)}`;
  parseVless(link);
  return link;
}
