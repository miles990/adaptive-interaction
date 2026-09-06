import {expect, it} from 'vitest';
import {initialSession, reduce} from './src/aip/sessionClient';
import {stateHash} from './src/aip/canonical';
const state = () => ({characterId:'independent-probe',mood:{kind:'neutral',intensity:0},activity:'idle',attention:{kind:'none'},truth:{state:'none'},members:[],reducedMotion:false});
const wire = (s:unknown) => ({messageType:'state',sessionId:'session.home',payload:{kind:'snapshot',revision:1,sessionEpoch:1,state:s,hash:stateHash(s)}});
const receive = (s:unknown) => reduce(initialSession(),{kind:'sse',arrivedOn:0,envelope:wire(s)}).next;
it('control: genuine complete snapshot is accepted',()=>{const result=receive(state());expect(result.lastDecision?.decision).toBe('apply');expect(result.local?.state).toEqual(state());});
for(const key of ['attention','members','reducedMotion']) it(`independent: missing ${key} cannot become local state`,()=>{
 const input:Record<string,unknown>=state();delete input[key];const before=JSON.stringify(input);const result=receive(input);
 console.log(JSON.stringify({probe:'missing-'+key,hash:stateHash(input),decision:result.lastDecision?.decision,adopted:result.local!==null}));
 expect(JSON.stringify(input)).toBe(before);expect(result.lastDecision?.decision).toBe('reject-invalid');expect(result.local).toBeNull();
});
it('independent: hash-consistent explicit optional null is not valid complete state',()=>{
 const input={...state(),lastInteraction:null};const result=receive(input);
 console.log(JSON.stringify({probe:'explicit-null',hash:stateHash(input),decision:result.lastDecision?.decision,adopted:result.local!==null}));
 expect(result.lastDecision?.decision).toBe('reject-invalid');expect(result.local).toBeNull();
});
it('independent: hash-consistent deletion of required members is rejected atomically',()=>{
 const original=receive(state());const before=JSON.stringify(original.local);const invalid:Record<string,unknown>=state();delete invalid.members;
 const result=reduce(original,{kind:'sse',arrivedOn:0,envelope:{messageType:'state',sessionId:'session.home',baseRevision:1,payload:{kind:'patch',revision:2,sessionEpoch:1,patch:{members:null},hash:stateHash(invalid)}}}).next;
 console.log(JSON.stringify({probe:'delete-members',hash:stateHash(invalid),decision:result.lastDecision?.decision,revision:result.local?.revision}));
 expect(result.lastDecision?.decision).toBe('reject-invalid');expect(result.local).toBe(original.local);expect(JSON.stringify(original.local)).toBe(before);
});
it('control: merge-patch null may delete an optional unknown extension',()=>{
 const original=receive({...state(),optionalExtension:'fixture'});
 const result=reduce(original,{kind:'sse',arrivedOn:0,envelope:{messageType:'state',sessionId:'session.home',baseRevision:1,payload:{kind:'patch',revision:2,sessionEpoch:1,patch:{optionalExtension:null},hash:stateHash(state())}}}).next;
 expect(result.lastDecision?.decision).toBe('apply');expect(result.local?.revision).toBe(2);expect(result.local?.state).toEqual(state());
});
