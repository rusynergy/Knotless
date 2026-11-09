import { fragmentMask, type FragmentSettings } from './fragment-config';
export type Outbound = Record<string, unknown>;
export type Server = { id: string; name: string; host: string; port: number; protocol: string; transport: string; security: string; outbound: Outbound; subscriptionId?: string };
export type Rule = { type: 'field'; outboundTag: string; domain?: string[]; ip?: string[]; port?: string; network?: string };
export const DEFAULT_RULES = '# Правила применяются сверху вниз\n# Действия: proxy, direct, block\ndefault: proxy\n\nip(geoip:private) -> direct\n';
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
export function needsLegacyCoreHint(server: Server): boolean {
  // parseVless currently accepts only encryption=none.
  if (server.security !== 'none') return false;
  let host = server.host;
  if (host.includes(':')) {
    // URL parsing canonicalizes IPv6 literals, including IPv4-mapped addresses.
    const mapped = /^::ffff:([\da-f]+):([\da-f]+)$/i.exec(host);
    if (!mapped) return host !== '::1' && !/^(?:f[cd]|fe[89ab])/i.test(host);
    const high = parseInt(mapped[1], 16), low = parseInt(mapped[2], 16);
    host = [high >> 8, high & 255, low >> 8, low & 255].join('.');
  }
  if (!/^\d{1,3}(?:\.\d{1,3}){3}$/.test(host)) return false;
  const octets = host.split('.').map(Number);
  if (octets.some(n => n > 255)) return false;
  const [a,b] = octets;
  return !(a === 10 || a === 127 || (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) || (a === 169 && b === 254));
}
export function parseVless(text: string): Server {
  let u: URL;
  try { u = new URL(text.trim()); } catch { throw new Error('Некорректная ссылка сервера.'); }
  if (u.protocol !== 'vless:') throw new Error('В этой версии поддерживаются ссылки vless://.');
  const id = decodeURIComponent(u.username);
  if (!uuid.test(id)) throw new Error('В ссылке должен быть корректный UUID VLESS.');
  const host = u.hostname.replace(/^\[|\]$/g, '');
  const port = Number(u.port || 443);
  if (!host || !Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Некорректный адрес или порт.');
  const p = u.searchParams;
  const transport = p.get('type') || 'tcp';
  if (!['tcp', 'raw', 'ws', 'grpc', 'xhttp', 'httpupgrade'].includes(transport)) throw new Error(`Транспорт ${transport} пока не поддерживается.`);
  const security = p.get('security') || 'none';
  if (!['none', 'tls', 'reality'].includes(security)) throw new Error(`Неизвестный тип защиты: ${security}.`);
  if (p.get('allowInsecure') === '1' || p.get('insecure') === '1') throw new Error('Отключение проверки TLS не поддерживается.');
  if (p.get('encryption') && p.get('encryption') !== 'none') throw new Error('Поддерживается encryption=none.');
  const stream: Record<string, unknown> = { network: transport, security };
  if (security === 'tls') stream.tlsSettings = { serverName: p.get('sni') || host, fingerprint: p.get('fp') || 'chrome', ...(p.get('alpn') ? { alpn: p.get('alpn')!.split(',') } : {}) };
  if (security === 'reality') {
    if (!p.get('pbk') || !p.get('sni')) throw new Error('Для REALITY нужны параметры pbk и sni.');
    stream.realitySettings = { serverName: p.get('sni'), fingerprint: p.get('fp') || 'chrome', publicKey: p.get('pbk'), shortId: p.get('sid') || '', spiderX: p.get('spx') || '/' };
  }
  if (transport === 'ws') stream.wsSettings = { path: p.get('path') || '/', headers: p.get('host') ? { Host: p.get('host') } : {} };
  if (transport === 'grpc') stream.grpcSettings = { serviceName: p.get('serviceName') || '', multiMode: p.get('mode') === 'multi' };
  if (transport === 'xhttp') {
    let extra: Record<string, unknown> = {};
    if (p.has('extra')) {
      try { extra = JSON.parse(p.get('extra')!); }
      catch { throw new Error('Параметр XHTTP extra должен содержать JSON-объект.'); }
      if (!extra || typeof extra !== 'object' || Array.isArray(extra) ||
          Object.keys(extra).some(key => !['mode', 'xPaddingBytes'].includes(key))) {
        throw new Error('В XHTTP extra поддерживаются mode и xPaddingBytes.');
      }
    }
    const mode = p.get('mode') || extra.mode || 'auto';
    if (!['auto', 'packet-up', 'stream-up', 'stream-one'].some(value => value === mode)) throw new Error('Некорректный режим XHTTP.');
    const padding = extra.xPaddingBytes ?? p.get('x_padding_bytes');
    if (padding != null && (typeof padding !== 'string' || !/^\d+(?:-\d+)?$/.test(padding))) throw new Error('Некорректный xPaddingBytes.');
    stream.xhttpSettings = { path: p.get('path') || '/', host: p.get('host') || '', mode, ...(padding != null ? { xPaddingBytes: padding } : {}) };
  }
  if (transport === 'httpupgrade') stream.httpupgradeSettings = { path: p.get('path') || '/', host: p.get('host') || host };
  if (['tcp', 'raw'].includes(transport) && p.get('headerType') && p.get('headerType') !== 'none') throw new Error('TCP headerType пока не поддерживается.');
  return { id: crypto.randomUUID(), name: decodeURIComponent(u.hash.slice(1)) || host, host, port, protocol: 'VLESS', transport, security,
    outbound: { tag: 'proxy', protocol: 'vless', settings: { vnext: [{ address: host, port, users: [{ id, encryption: 'none', ...(p.get('flow') ? { flow: p.get('flow') } : {}) }] }] }, streamSettings: stream } };
}
export function compileRouting(source: string): { defaultTag: string; rules: Rule[] } {
  let defaultTag = 'proxy'; let hasDefault = false;
  const rules: Rule[] = [];
  source.split(/\r?\n/).forEach((raw, index) => {
    const line = raw.trim(); if (!line || line.startsWith('#')) return;
    const fail = (message: string): never => { throw new Error(`Строка ${index + 1}: ${message}`); };
    const def = /^default:\s*(proxy|direct|block)$/.exec(line);
    if (def) { if (hasDefault) fail('default указан дважды.'); hasDefault = true; defaultTag = def[1]; return; }
    const match = /^(.+)\s*->\s*(proxy|direct|block)$/.exec(line);
    if (!match) fail('ожидается domain(...), ip(...), port(...) или network(...) -> действие.');
    const [, conditions, target] = match!;
    const rule: Rule = { type: 'field', outboundTag: target };
    // Split conjunctions only outside condition parentheses; regex groups and
    // escaped parentheses inside domain expressions remain intact.
    const parts:string[]=[]; let start=0, depth=0, escaped=false, inClass=false;
    for(let i=0;i<conditions.length;i++){
      const c=conditions[i];
      if(escaped){escaped=false;continue;} if(c==='\\'){escaped=true;continue;}
      if(c==='[')inClass=true; if(c===']')inClass=false; if(inClass)continue;
      if(c==='(')depth++; if(c===')')depth--;
      if(depth<0)fail('лишняя закрывающая скобка.');
      if(depth===0 && conditions.slice(i,i+2)==='&&'){parts.push(conditions.slice(start,i).trim());start=i+2;i++;}
    }
    if(depth!==0)fail('незакрытая скобка.');
    parts.push(conditions.slice(start).trim());
    for(const part of parts){
    const condition=/^(domain|ip|port|network)\((.+)\)$/.exec(part);
    if(!condition)fail('ожидается условие domain(...), ip(...), port(...) или network(...); условия соединяются через &&.');
    const [,kind,rawValue]=condition!; const value=rawValue.trim();
    if(Object.prototype.hasOwnProperty.call(rule,kind))fail(`повтор условия ${kind} в одной строке не поддерживается.`);
    if (!value) fail('пустое условие.');
    if (kind === 'domain') {
      // A separator introduces another prefixed matcher. Commas in regex
      // quantifiers, e.g. {1,3}, must not split the expression.
      rule.domain = value.split(/,\s*(?=(?:domain|full|contains|regexp|geosite)\s*:)/).map(item=>{
        const v=item.trim().replace(/^([a-z]+):\s*/, '$1:');
        if (!/^(domain|full|contains|regexp|geosite):\S/.test(v)) fail('нужен префикс domain:, full:, contains:, regexp: или geosite:.');
        if(!v.startsWith('regexp:') && /[,\s]/.test(v))fail('некорректный элемент списка доменов: у каждого элемента нужен префикс.');
        return v.replace(/^contains:/, 'keyword:');
      });
    }
    if (kind === 'ip') {
      const values = value.split(',').map(item => item.trim());
      for (const item of values) {
        if (!item) fail('пустой элемент в списке IP-условий.');
        if (!/^(geoip:[\w!-]+|[a-fA-F0-9:.]+(?:\/\d{1,3})?)$/.test(item)) fail(`некорректное IP-условие: ${item}.`);
      }
      rule.ip = values;
    }
    if (kind === 'port') {
      const ports=value.split(',').map(item=>item.trim());
      if (ports.some(item=>!/^\d+(?:\s*-\s*\d+)?$/.test(item))) fail('некорректные порты.');
      for (const range of ports) { const n = range.split('-').map(Number); if (n.some(x => x < 1 || x > 65535) || (n.length === 2 && n[0] > n[1])) fail('порт вне диапазона.'); }
      rule.port = ports.map(item=>item.replace(/\s/g,'')).join(',');
    }
    if (kind === 'network') { const networks=value.split(',').map(item=>item.trim()).join(',');if (!/^(tcp|udp|tcp,udp|udp,tcp)$/.test(networks)) fail('ожидается tcp или udp.'); rule.network = networks; }
    }
    rules.push(rule);
  });
  return { defaultTag, rules };
}
export function buildConfig(server: Server, source: string, probe = false, fragment?: FragmentSettings): Record<string, unknown> {
  const route = probe ? { defaultTag: 'proxy', rules: [] } : compileRouting(source);
  const proxy = fragment?.enabled ? { ...server.outbound, streamSettings: { ...(server.outbound.streamSettings as object), finalmask: fragmentMask(fragment) } } : server.outbound;
  const outbounds = [proxy, { tag: 'direct', protocol: 'freedom' }, { tag: 'block', protocol: 'blackhole' }];
  outbounds.sort((a, b) => Number(b.tag === route.defaultTag) - Number(a.tag === route.defaultTag));
  return { log: { loglevel: 'warning' }, inbounds: [], outbounds: probe ? [proxy] : outbounds,
    routing: { domainStrategy: 'AsIs', rules: route.rules } };
}
