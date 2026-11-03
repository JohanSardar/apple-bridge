import base64
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
from matrix import control


def run(peer, lab):
    with tempfile.TemporaryDirectory(prefix='gb-query-') as directory:
        home=Path(directory)
        secret=home/'secret'
        secret.write_bytes(bytes([7]*32))
        secret.chmod(0o600)
        server=subprocess.Popen([peer,str(home/'peer.sock')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True,env=dict(os.environ,AB_PROBE_BLOB_CAP='512'))
        apple=subprocess.Popen([lab,'simapple','--socket',str(home/'peer.sock'),'--secret-file',str(secret),'--control',str(home/'control.sock')],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
        def exchange(value):
            server.stdin.write(json.dumps(value)+'\n');server.stdin.flush()
            line=server.stdout.readline()
            assert line,(server.poll(),apple.poll())
            return json.loads(line)
        def query(op,args,ident='qry_case',deadline=1000):
            return exchange({'t':'LocalQuery','args':{'id':ident,'op':op,'args':args,'deadline_ms':deadline}})
        results={}
        def code(value):return value.get('error',{}).get('code')
        try:
            deadline=time.monotonic()+20
            while time.monotonic()<deadline:
                if exchange({'t':'Available'}).get('available') and (home/'control.sock').exists():break
                time.sleep(.02)
            else:raise AssertionError('not ready')
            for op,args in [('helper.version',{}),('test.echo',{'text':'<&> Synthetic'}),('test.echo',[1,True,'Synthetic']),('test.echo',None),('mac.network_check',{}),('mac.bluetooth_devices',{})]:
                value=query(op,args)
                results[op+':'+json.dumps(args,sort_keys=True)]=value
            for op,args in [('helper.version',{'unexpected':True}),('mac.network_check',None),('mac.recognize_song',{'audio_blob':''}),('mail.body',{}),('mail.body',{'message_id':'missing'}),('vision.ocr',{'file':'/synthetic/missing'}),('vision.ocr',{'file':'/synthetic/missing','allow_file':False}),('vision.ocr',{'file':'/synthetic/missing','allow_file':True,'data_base64':'YWJj'}),('vision.ocr',{'blob':{'query_id':'qry_missing','digest':'0'*64}}),('vision.ocr',{'blob':{'query_id':'qry_missing','digest':'0'*64},'data_base64':'YWJj'})]:
                results[op+':'+json.dumps(args,sort_keys=True)]=query(op,args)
            value=query('test.sleep',{'ms':250},ident='qry_deadline',deadline=5)
            assert code(value)=='deadline',value
            time.sleep(.05)
            assert control(str(home/'control.sock'),'GET','/effects').get('qry_deadline',0)==0
            results['deadline']={'code':code(value)}
            results['after_deadline']=query('helper.version',{},ident='qry_after_deadline')
            data=b'%PDF-Synthetic\n'+bytes(180000)
            value=query('pdf.text',{'data_base64':base64.b64encode(data).decode()},ident='qry_large_document')
            assert code(value)=='unavailable',value
            results['large_document']=value
            assert not list(Path(tempfile.gettempdir()).glob('ab-document-*'))
            assert not list((home/'gateway-blobs'/'blobs'/'document-inputs').glob('input-*'))
            local_file=home/'synthetic.pdf'
            local_file.write_bytes(data)
            value=query('pdf.text',{'file':str(local_file),'allow_file':True},ident='qry_allowed_file')
            assert code(value)=='unavailable',value
            results['allowed_file']=value
            value=query('speech.synthesize',{'text':'Bridge fixture'},ident='qry_audio')
            assert value.get('blobs') and 'handle' not in value['blobs'][0],value
            results['audio']=value
            capacity=query('test.blob',{'size':1024},ident='qry_capacity')
            assert code(capacity)=='capacity',capacity
            results['capacity']=capacity
            digest=value['blobs'][0]['digest']
            value=query('vision.ocr',{'blob':{'query_id':'qry_audio','digest':digest}},ident='qry_blob_input')
            assert code(value)=='invalid',value
            results['blob_input']=value
            value=query('vision.ocr',{'blob':{'query_id':'qry_audio','digest':'0'*64}},ident='qry_wrong_digest')
            assert code(value)=='unauthorized',value
            results['wrong_digest']=value
            import sqlite3
            with sqlite3.connect(home/'gateway-state'/'state.db') as db:db.execute("UPDATE query_blobs SET expires=0 WHERE id='qry_audio'")
            value=query('vision.ocr',{'blob':{'query_id':'qry_audio','digest':digest}},ident='qry_expired_grant')
            assert code(value)=='forbidden',value
            results['expired_grant']=value
            control(str(home/'control.sock'),'POST','/blob-fixture',{'size':1,'state':'offloaded'})
            value=query('speech.synthesize',{'text':'Bridge fixture'},ident='qry_offloaded')
            assert code(value)=='unsupported',value
            results['offloaded']=value
            control(str(home/'control.sock'),'POST','/blob-fixture',{'size':1,'state':'present','slow_ms':150})
            value=query('speech.synthesize',{'text':'Bridge fixture'},ident='qry_slow_blob',deadline=20)
            assert code(value)=='deadline',value
            results['blob_deadline']={'code':code(value)}
            time.sleep(.2)
            control(str(home/'control.sock'),'POST','/blob-fixture',{'size':1,'state':'present'})
            results['after_blob_deadline']=query('helper.version',{},ident='qry_after_blob_deadline')
            seed=exchange({'t':'Query','id':'qry_seed_blob','op':'test.blob','args':{'size':3},'deadline_ms':1000})
            ref=seed['blobs'][0]
            assert exchange({'t':'StartBlob','handle':ref['handle'],'length':3})=={'started':True}
            apple.terminate();apple.wait(timeout=5)
            deadline=time.monotonic()+10
            while exchange({'t':'Available'}).get('available') and time.monotonic()<deadline:time.sleep(.01)
            assert not exchange({'t':'Available'}).get('available')
            apple=subprocess.Popen([lab,'simapple','--socket',str(home/'peer.sock'),'--secret-file',str(secret),'--control',str(home/'control.sock')],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
            deadline=time.monotonic()+15
            while not exchange({'t':'Available'}).get('available') and time.monotonic()<deadline:time.sleep(.02)
            assert exchange({'t':'Available'}).get('available')
            stale=exchange({'t':'FinishBlob'})
            assert stale.get('error',{}).get('code')=='unavailable',stale
            results['stale_blob_session']=stale
            fresh=exchange({'t':'Query','id':'qry_fresh_blob','op':'test.blob','args':{'size':3},'deadline_ms':1000})
            ref=fresh['blobs'][0]
            fresh=exchange({'t':'ReadBlob','handle':ref['handle'],'length':3})
            import hashlib
            data=base64.b64decode(fresh['bytes'])
            assert len(data)==3 and hashlib.sha256(data).hexdigest()==ref['digest'],fresh
            results['fresh_blob_session']=fresh
            audits=exchange({"t":"QueryAudits"})
            staged=next(a for a in audits if a['operation']=='pdf.text')
            assert 0<staged['argument_bytes']<512,staged
            staged['argument_bytes']='<generated staged-file path>'
            results['audit']=audits
            return results
        finally:
            apple.terminate();server.stdin.close()
            try:apple.wait(timeout=5)
            except subprocess.TimeoutExpired:apple.kill();apple.wait()
            try:server.wait(timeout=5)
            except subprocess.TimeoutExpired:server.kill();server.wait()

def rate(peer):
    with tempfile.TemporaryDirectory(prefix='gb-rate-') as directory:
        process=subprocess.Popen([peer,str(Path(directory)/'peer.sock')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        try:
            outcomes=[]
            started=time.monotonic()
            for _ in range(102):
                process.stdin.write(json.dumps({'t':'LocalQuery','args':{'id':'qry_rate','op':'helper.version','args':{'unexpected':True}}})+'\n')
                process.stdin.flush()
                outcomes.append(json.loads(process.stdout.readline()).get('error',{}).get('code'))
            elapsed=time.monotonic()-started
            assert elapsed<1,(elapsed,'rate fixture exceeded its window')
            assert outcomes==['invalid']*100+['capacity']*2,outcomes
            return {'invalid':100,'capacity':2}
        finally:
            process.stdin.close()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()

if __name__=='__main__':
    go,rust,lab=sys.argv[1:]
    expected=run(go,lab)
    actual=run(rust,lab)
    assert actual==expected,{key:{'go':expected[key],'rust':actual.get(key)} for key in expected if expected[key]!=actual.get(key)}
    print(json.dumps({'queries':len(expected),'go':expected,'rust':actual,'match':True,'rust_rate':rate(rust)}),flush=True)
