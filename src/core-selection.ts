import type { Core } from './bridge';
export function selectInitialCore(current:string,cores:Core[],seenBundle:string):string{
 const bundled=cores.find(c=>c.bundled)?.version;
 if(bundled&&bundled!==seenBundle)return bundled;
 return cores.some(c=>c.version===current)?current:bundled||cores[0]?.version||'';
}
