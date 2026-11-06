import { invoke, isTauri } from '@tauri-apps/api/core';
export const desktop = isTauri();
export async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) throw new Error('Эта операция доступна в Windows-приложении Knotless. В браузере открыт просмотр интерфейса.');
  return invoke<T>(name, args);
}
export type Core = { version: string; path: string; bundled?:boolean };
export type RuntimeStatus = { running: boolean; core: string | null; socksPort: number; httpPort: number; mode: string; name: string; busy: boolean; elevated: boolean };
export type Probe = { ms: number; status: number; url: string };
export type Telemetry = {session:string;downRate:number;upRate:number;downloaded:number;uploaded:number;available:boolean;health:string;detail:string;checkedAt:number};
