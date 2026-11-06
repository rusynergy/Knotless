import { describe, expect, it } from 'vitest';
import { parseVless, compileRouting, buildConfig } from './domain';
const link = 'vless://4ce82b94-cf44-4415-879c-7a8c29925475@example.com:443?security=reality&sni=example.com&pbk=abc&fp=chrome&type=tcp#Test';
describe('VLESS import', () => {
  it('retains REALITY settings and name', () => { const s = parseVless(link); expect(s.name).toBe('Test'); expect(s.outbound.streamSettings).toMatchObject({ realitySettings: { publicKey: 'abc', serverName: 'example.com' } }); });
  it('rejects unsupported and insecure links instead of silently changing semantics', () => { expect(() => parseVless(link + '&')).not.toThrow(); expect(() => parseVless(link.replace('type=tcp', 'type=kcp'))).toThrow(); expect(() => parseVless(link.replace('fp=chrome', 'allowInsecure=1'))).toThrow(); expect(() => parseVless(link.replace('pbk=abc&', ''))).toThrow(); });
});
describe('routing', () => {
  it('compiles comma-separated CIDRs into one Xray rule', () => {
    const source = 'ip(91.108.56.0/22, 91.108.4.0/22, 91.108.8.0/22, 91.108.16.0/22, 91.108.12.0/22, 149.154.160.0/20, 91.105.192.0/23, 91.108.20.0/22, 185.76.151.0/24)->proxy';
    const expected = [{ type: 'field', outboundTag: 'proxy', ip: ['91.108.56.0/22', '91.108.4.0/22', '91.108.8.0/22', '91.108.16.0/22', '91.108.12.0/22', '149.154.160.0/20', '91.105.192.0/23', '91.108.20.0/22', '185.76.151.0/24'] }];
    expect(compileRouting(source).rules).toEqual(expected);
    expect(buildConfig(parseVless(link), source).routing).toEqual({ domainStrategy: 'AsIs', rules: expected });
  });
  it('accepts whitespace, IPv6, individual IPs and geoip in an IP list', () => {
    expect(compileRouting('ip( geoip:private ,\t2001:db8::/32, 1.1.1.1 )->direct').rules[0].ip).toEqual(['geoip:private', '2001:db8::/32', '1.1.1.1']);
  });
  it('rejects empty or malformed list entries without dropping them', () => {
    for (const value of [',1.1.1.1', '1.1.1.1,', '1.1.1.1,,8.8.8.8', '1.1.1.1, example.com']) {
      expect(() => compileRouting(`# comment\nip(${value})->proxy`)).toThrow('Строка 2');
    }
  });
  it('keeps rule order and translates contains', () => { expect(compileRouting('domain(contains: ads) -> block\nip(geoip:private) -> direct\ndefault: proxy').rules).toEqual([{ type: 'field', outboundTag: 'block', domain: ['keyword:ads'] }, { type: 'field', outboundTag: 'direct', ip: ['geoip:private'] }]); });
  it('fails on invalid rules and duplicate defaults', () => { expect(() => compileRouting('invalid -> direct')).toThrow('Строка 1'); expect(() => compileRouting('port(8000-1) -> proxy')).toThrow(); expect(() => compileRouting('default: direct\ndefault: proxy')).toThrow(); });
  it('forces probe through proxy even if user routing bypasses or blocks everything', () => { const s = parseVless(link); const c = buildConfig(s, 'default: direct\nnetwork(tcp) -> block', true); expect(c.outbounds).toEqual([s.outbound]); expect(c.routing).toEqual({ domainStrategy: 'AsIs', rules: [] }); });
  it('places default outbound first', () => { expect((buildConfig(parseVless(link), 'default: block').outbounds as {tag:string}[])[0].tag).toBe('block'); });
});
