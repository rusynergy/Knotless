import {expect,it} from 'vitest';
import {selectInitialCore} from './core-selection';
const cores=[{version:'v1',path:'old'},{version:'v2',path:'bundle',bundled:true}];
it('selects a bundled core on first launch and a new bundle after update',()=>{expect(selectInitialCore('',cores,'')).toBe('v2');expect(selectInitialCore('v1',cores,'v1')).toBe('v2');});
it('preserves a manual selection once the bundle has been seen',()=>{expect(selectInitialCore('v1',cores,'v2')).toBe('v1');expect(selectInitialCore('missing',cores,'v2')).toBe('v2');});
