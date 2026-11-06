import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
const out = mkdtempSync(join(tmpdir(), 'purin-bbox-'));
after(() => rmSync(out, { recursive: true, force: true }));
execFileSync(process.execPath, [join(root, 'node_modules/typescript/bin/tsc'), 'src/utils/bboxRegions.ts', '--outDir', out, '--target', 'ES2020', '--module', 'commonjs', '--skipLibCheck'], { cwd: root });
const { bboxRegions } = createRequire(import.meta.url)(join(out, 'bboxRegions.js'));
test('GBC extracts edge names, normalized bounds and nested styling', () => {
 const bbox = {left:.1,top:.2,right:.8,bottom:.9};
 const graph = {vertices:[
  {vertex_id:'',label:'image',bbox,out_edges:[{source:'',target:'girl',text:'girl'}]},
  {vertex_id:'girl',label:'entity',bbox,out_edges:[{source:'girl',target:'cloak',text:'red hooded cloak'}]},
  {vertex_id:'cloak',label:'entity',bbox},
  {vertex_id:'relation',label:'relation',bbox},
 ]};
 const result=bboxRegions({graph});
 assert.equal(result.length,2); assert.equal(result[0].label,'girl');
 assert.deepEqual(result[1].box,[.1,.2,.8,.9]); assert.equal(result[1].label,'red hooded cloak');
 assert.equal(result[1].nested,true);assert.notEqual(result[1].color,result[0].color);
});
test('MOSS absolute boxes use declared canvas, preserving text',()=>{
 const result=bboxRegions({width:1600,height:800,bbox_coord_space:'abs',bbox_format:'xyxy',items:[{label:'text',text:'Hello 世界',caption:'sign',bbox:[160,80,800,400]}]});
 assert.deepEqual(result[0].box,[.1,.1,.5,.5]);assert.equal(result[0].label,'Hello 世界');
});
test('ordinary tags, invalid rectangles and unknown coordinate conventions produce no boxes',()=>{
 for (const value of [null,{tags:['girl']},{vertices:[null,{label:'entity',bbox:{left:1,top:0,right:0,bottom:1}}]}, {width:10,height:10,bbox_coord_space:'rel',bbox_format:'xyxy',items:[{bbox:[0,0,1,1]}]}]) assert.deepEqual(bboxRegions(value),[]);
});

test('nine regions keep distinct colors in both formats',()=>{
 const vertices=Array.from({length:9},(_,i)=>({vertex_id:String(i),label:'entity',bbox:{left:0,top:0,right:1,bottom:1}}));
 const items=vertices.map((v,i)=>({label:'object',caption:String(i),bbox:[0,0,100,100]}));
 for(const doc of [{vertices},{width:100,height:100,bbox_coord_space:'abs',bbox_format:'xyxy',items}]) {
  const result=bboxRegions(doc);assert.equal(new Set(result.map(r=>r.color)).size,9);
  assert.deepEqual(bboxRegions(doc),result);
 }
});
