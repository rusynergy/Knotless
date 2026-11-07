export type FragmentSettings = { enabled: boolean; packets: 'tlshello' | '1-3'; length: string; delay: string };
export const defaultFragment: FragmentSettings = { enabled: false, packets: 'tlshello', length: '100-200', delay: '1-3' };

function range(text: string, min: number, max: number): string | number {
  const m = /^(\d+)(?:\s*-\s*(\d+))?$/.exec(text.trim());
  if (!m) throw new Error('fragInvalid');
  const low = Number(m[1]), high = Number(m[2] ?? m[1]);
  if (low < min || high > max || low > high) throw new Error('fragInvalid');
  return low === high ? low : `${low}-${high}`;
}
export function fragmentMask(value: FragmentSettings) {
  if (!['tlshello','1-3'].includes(value.packets)) throw new Error('fragInvalid');
  return { tcp: [{ type: 'fragment', settings: { packets: value.packets, lengths: [range(value.length, 1, 16384)], delays: [range(value.delay, 0, 1000)] } }] };
}
