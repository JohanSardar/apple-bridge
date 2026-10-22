import test from 'node:test';
import assert from 'node:assert/strict';
import {transcriptChunks} from './message-chunks.mjs';
test('transcript grouping excludes shell modules and first-use dynamic descendants', () => {
  const root='/project/web/',pane=root+'src/messages/MessagePane.tsx',composer=root+'src/messages/compose/Composer.tsx',dock=root+'src/messages/compose/UploadDock.tsx';
  const nodes=new Map([
    ['index.html',{isEntry:true,importedIds:['shell','shared'],dynamicallyImportedIds:[pane]}],
    ['shell',{importedIds:['shared']}],['shared',{importedIds:[]}],
    [pane,{importedIds:['shared','transcript',composer,dock]}],
    ['transcript',{importedIds:['media'],dynamicallyImportedIds:['history']}],
    ['media',{importedIds:['transcript'],dynamicallyImportedIds:['viewer']}],
    [composer,{importedIds:['shared']}],[dock,{importedIds:['shared']}],
    ['viewer',{importedIds:['shared']}],['history',{importedIds:['shared']}]
  ]);
  const chunk=transcriptChunks(),graph={getModuleIds:()=>nodes.keys(),getModuleInfo:id=>nodes.get(id)};
  assert.deepEqual([...nodes.keys()].filter(id=>chunk(id,graph)),['transcript','media']);
});
