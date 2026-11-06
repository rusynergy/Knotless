import { describe, it, expect } from 'vitest';
import { subscriptionServers, mergeSubscriptionServers, type Subscription } from './subscriptions';
import { parseVless } from './domain';
import { languages, translator } from './i18n';
const link='vless://11111111-1111-4111-8111-111111111111@example.com:443?security=tls#One';
const sub:Subscription={id:'sub1',name:'Test',url:'https://example.com/sub',sendHwid:true,autoUpdate:true,intervalHours:24,content:link,updatedAt:0,attemptedAt:0,error:null};
describe('subscriptions',()=>{
  it('keeps selection stable across refreshes and renamed servers',async()=>{const first=await subscriptionServers(sub);const next=await subscriptionServers({...sub,content:link.replace('#One','#Renamed')});expect(first.servers[0].id).toBe(next.servers[0].id);expect(next.servers[0].name).toBe('Renamed');});
  it('deduplicates and reports unsupported links',async()=>{const result=await subscriptionServers({...sub,content:`${link}\n${link}\nss://unsupported`});expect(result.servers).toHaveLength(1);expect(result.skipped).toBe(1);});
  it('preserves manual servers and failed subscriptions, removes deleted groups',async()=>{const manual=parseVless(link);const a=(await subscriptionServers(sub)).servers;const b=(await subscriptionServers({...sub,id:'sub2'})).servers;const current=[manual,...a,...b];expect(mergeSubscriptionServers(current,['sub1'],new Map())).toEqual([manual,...a]);const changed={...a[0],name:'Updated'};expect(mergeSubscriptionServers(current,['sub1','sub2'],new Map([['sub1',[changed]]]))).toEqual([manual,...b,changed]);});
  it('rejects unusable content instead of replacing existing servers',async()=>{await expect(subscriptionServers({...sub,content:'vless://bad'})).rejects.toThrow('SUB_NO_VLESS');});
});
it('has distinct UI translations for every supported language',()=>{expect(languages.map(l=>translator(l)('settings'))).toEqual(['Настройки','Settings','设置']);});
