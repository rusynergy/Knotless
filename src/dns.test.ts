import { expect,it } from 'vitest';
import { defaultDns,dnsProviders,validDns } from './dns';
it('defaults to Quad9 and validates all presets',()=>{expect(defaultDns.address).toBe('9.9.9.9');for(const provider of dnsProviders){expect(validDns({...provider,ipv6:false,transport:'tcp'})).toBe(true);expect(validDns({...provider,ipv6:false,transport:'doh'})).toBe(true);}});
it('rejects malformed DNS settings',()=>{for(const address of ['x','999.1.1.1','1.2.3','01.2.3.4'])expect(validDns({...defaultDns,address})).toBe(false);for(const dohUrl of ['http://dns.google/dns-query','https://user:pass@example.com/dns-query','https://example.com/#fragment'])expect(validDns({...defaultDns,transport:'doh',dohUrl})).toBe(false);expect(validDns({...defaultDns,address:'2620:fe::fe'})).toBe(true);});
