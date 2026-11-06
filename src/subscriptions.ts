import { parseVless, type Server } from './domain';
export type Subscription = {id:string;name:string;url:string;sendHwid:boolean;autoUpdate:boolean;intervalHours:number;content:string;updatedAt:number;attemptedAt:number;error:string|null};
export async function subscriptionServers(sub:Subscription):Promise<{servers:Server[];skipped:number}> {
  const servers:Server[]=[];let skipped=0;const ids=new Set<string>();
  for(const line of sub.content.split(/\r?\n/).map(s=>s.trim()).filter(Boolean)){
    try{const server=parseVless(line);const hash=await crypto.subtle.digest('SHA-256',new TextEncoder().encode(JSON.stringify(server.outbound)));const id=`${sub.id}:${Array.from(new Uint8Array(hash),b=>b.toString(16).padStart(2,'0')).join('')}`;if(ids.has(id))continue;ids.add(id);servers.push({...server,id,subscriptionId:sub.id});}catch{skipped++;}
  }
  if(!servers.length)throw new Error('SUB_NO_VLESS');return {servers,skipped};
}
export function mergeSubscriptionServers(current:Server[],knownIds:string[],updates:Map<string,Server[]>):Server[]{
  return [...current.filter(s=>!s.subscriptionId||(knownIds.includes(s.subscriptionId)&&!updates.has(s.subscriptionId))),...Array.from(updates.values()).flat()];
}
