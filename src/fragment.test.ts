import { expect, it } from 'vitest';
import { buildConfig, parseVless } from './domain';
import { fragmentMask, defaultFragment } from './fragment-config';
const server = parseVless('vless://11111111-1111-4111-8111-111111111111@example.com:443?security=tls');
it('changes only the proxy leg and uses the same transport in the HTTPS test',()=>{
  const options={...defaultFragment,enabled:true};
  const config=buildConfig(server,'default: direct',false,options);
  const probe=buildConfig(server,'',true,options);
  expect(config.outbounds).toEqual([{tag:'direct',protocol:'freedom'},...(probe.outbounds as object[]),{tag:'block',protocol:'blackhole'}]);
  expect(probe.outbounds).toMatchObject([{streamSettings:{security:'tls',finalmask:fragmentMask(options)}}]);
  expect(buildConfig(server,'',true).outbounds).toEqual([server.outbound]);
  expect(server.outbound.streamSettings).not.toHaveProperty('finalmask');
});
it('accepts zero delay and rejects empty, reversed and unbounded ranges',()=>{
  expect(fragmentMask({...defaultFragment,delay:'0'}).tcp[0].settings.delays).toEqual([0]);
  for(const length of ['','0','10-1','a','16385']) expect(()=>fragmentMask({...defaultFragment,length})).toThrow();
});
