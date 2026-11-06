export type Outbound = Record<string, unknown>;
export type Server = { id: string; name: string; host: string; port: number; protocol: string; transport: string; security: string; outbound: Outbound; subscriptionId?: string };
export type Rule = { type: 'field'; outboundTag: string; domain?: string[]; ip?: string[]; port?: string; network?: string };
export const DEFAULT_RULES = '# Правила применяются сверху вниз\n# Действия: proxy, direct, block\ndefault: proxy\n\nip(geoip:private) -> direct\n';
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
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
    if (p.has('extra')) throw new Error('Расширенные XHTTP extra пока не поддерживаются.');
    stream.xhttpSettings = { path: p.get('path') || '/', host: p.get('host') || '', mode: p.get('mode') || 'auto' };
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
    const match = /^(domain|ip|port|network)\((.+)\)\s*->\s*(proxy|direct|block)$/.exec(line);
    if (!match) fail('ожидается domain(...), ip(...), port(...) или network(...) -> действие.');
    const [, kind, rawValue, target] = match!; const value = rawValue.trim();
    if (!value) fail('пустое условие.');
    const rule: Rule = { type: 'field', outboundTag: target };
    if (kind === 'domain') {
      const v = value.replace(/^([a-z]+):\s*/, '$1:');
      if (!/^(domain|full|contains|regexp|geosite):.+/.test(v)) fail('нужен префикс domain:, full:, contains:, regexp: или geosite:.');
      rule.domain = [v.replace(/^contains:/, 'keyword:')];
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
      if (!/^\d+(?:-\d+)?(?:,\d+(?:-\d+)?)*$/.test(value)) fail('некорректные порты.');
      for (const range of value.split(',')) { const n = range.split('-').map(Number); if (n.some(x => x < 1 || x > 65535) || (n.length === 2 && n[0] > n[1])) fail('порт вне диапазона.'); }
      rule.port = value;
    }
    if (kind === 'network') { if (!/^(tcp|udp|tcp,udp|udp,tcp)$/.test(value)) fail('ожидается tcp или udp.'); rule.network = value; }
    rules.push(rule);
  });
  return { defaultTag, rules };
}
export function buildConfig(server: Server, source: string, probe = false): Record<string, unknown> {
  const route = probe ? { defaultTag: 'proxy', rules: [] } : compileRouting(source);
  const outbounds = [server.outbound, { tag: 'direct', protocol: 'freedom' }, { tag: 'block', protocol: 'blackhole' }];
  outbounds.sort((a, b) => Number(b.tag === route.defaultTag) - Number(a.tag === route.defaultTag));
  return { log: { loglevel: 'warning' }, inbounds: [], outbounds: probe ? [server.outbound] : outbounds,
    routing: { domainStrategy: 'AsIs', rules: route.rules } };
}
