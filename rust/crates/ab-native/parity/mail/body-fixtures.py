import base64
import hashlib
import json
import pathlib
import shlex
import shutil
import subprocess
import sys
import tempfile

oracle,binary=sys.argv[1:]
root_script=pathlib.Path(__file__).resolve().parents[5]
fixtures=root_script/'lab/fixtures/mail'
vectors=[]
def add(name,message,**extra):
    data=str(len(message)).encode()+b'\n'+message
    vectors.append(dict(name=name,input=base64.b64encode(data).decode(),partial=False,**extra))
for file in ['1.emlx','2.emlx','3.partial.emlx','4.emlx']:
    vectors.append(dict(name=file,fixture=file,partial='.partial.' in file))
multipart=b'Content-Type: multipart/alternative; boundary=fixture\n\n--fixture\nContent-Type: text/plain\n\n\n> [ab-test] fixture\n\n--fixture--\n'
add('MailBodyChecks-preserved',multipart)
vectors.append(dict(name='MailBodyChecks-padded',input=base64.b64encode(str(len(multipart)).ljust(10).encode()+b'\n'+multipart).decode(),partial=False))
add('MailBodyChecks-html-only',b'Content-Type: text/html\n\n<p>Hello</p><script>bad()</script>')
add('MailBodyChecks-latin1',b'Content-Type: text/plain; charset=iso-8859-1\n\nCaf\xe9')
vectors.append(dict(name='MailBodyChecks-large',repeat=1100000,prefix='Content-Type: text/plain\n\n',partial=False))
add('invalid-base64',b'Content-Transfer-Encoding: base64\n\nnot:base64')
add('base64-no-padding',b'Content-Transfer-Encoding: base64\n\nSGVsbG8')
add('base64-trailing-bits',b'Content-Transfer-Encoding: base64\n\nZh==')
add('base64-extra-padding',b'Content-Transfer-Encoding: base64\n\nSGVsbG8===')
for payload in ['=', '==', '===', '====', '========', 'A===', 'AA=', 'AA===', 'AAA==', 'AAAA====', '=AAA', 'A=AA', 'AA=A']:
    add('base64-shape-'+payload,b'Content-Transfer-Encoding: base64\n\n'+payload.encode())
add('bad-quoted-printable',b'Content-Transfer-Encoding: quoted-printable\n\nOne=20two=\nthree=ZZfour=')
add('unsupported-transfer',b'Content-Transfer-Encoding: other\n\npayload')
add('invalid-utf8',b'Content-Type: text/plain\n\n\x80')
add('html-scripts-and-styles',b'Content-Type: text/html\n\n<p>A &amp; B</p><SCRIPT type="x">\nx()</SCRIPT ><STYLE>x</STYLE><img src=x>')
add('attachment-only',b'Content-Disposition: attachment; filename=x\n\nignored')
add('missing-boundary',b'Content-Type: multipart/mixed\n\nignored')
add('empty-boundary',b'Content-Type: multipart/mixed; boundary=""\n\nignored')
add('folded-headers',b'Content-Type: text/plain;\r\n charset=iso-8859-1\r\nContent-Transfer-Encoding:\r\n quoted-printable\r\n\r\nCaf=E9')
add('latin-control',b'Content-Type: text/plain; charset=iso-8859-1\n\n\x80\x85\x9f')
vectors.append(dict(name='bad-length',input=base64.b64encode(b'not-a-length\nbody').decode(),partial=False))
vectors.append(dict(name='negative-length',input=base64.b64encode(b'-1\nbody').decode(),partial=False))
vectors.append(dict(name='declared-truncated',input=base64.b64encode(b'200\nContent-Type: text/plain\n\nShort').decode(),partial=False))
message=b'Content-Type: text/plain\n\ndeep'
for _ in range(16):message=b'Content-Type: multipart/mixed; boundary=x\n\n--x\n'+message+b'\n--x--\n'
add('mime-depth-limit',message)
remote=subprocess.check_output(['ssh','build','mktemp -d ~/w/mail-body-fixture-XXXXXX'],text=True).strip()
processes=[]
try:
    with tempfile.TemporaryDirectory(prefix='ab-mail-body-fixture-') as tmp:
        root=pathlib.Path(tmp);body=root/'bodies';body.mkdir()
        for index,vector in enumerate(vectors):
            if 'fixture' in vector:data=(fixtures/vector['fixture']).read_bytes()
            elif 'repeat' in vector:
                message=vector['prefix'].encode()+b'a'*vector['repeat'];data=str(len(message)).encode()+b'\n'+message
            else:data=base64.b64decode(vector['input'])
            filename=str(index+1)+('.partial' if vector['partial'] else '')+'.emlx';(body/filename).write_bytes(data)
        subprocess.run(['rsync','-acz',str(root)+'/', 'build:'+remote+'/'],check=True)
        processes=[subprocess.Popen([str(pathlib.Path.home()/'bin/ab-local'),oracle,str(root)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True),subprocess.Popen(['ssh','build',shlex.join([binary,'mail',remote,'--interactive','--frames'])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)]
        results=[];raw=[]
        for index,vector in enumerate(vectors):
            responses=[]
            for process in processes:
                process.stdin.write('body '+json.dumps({'message_id':str(index+1)})+'\n');process.stdin.flush();line=process.stdout.readline()
                if not line:raise RuntimeError('body reader exited')
                raw.append('{"observations":[{"data":'+line.strip()+'}]}\n');responses.append(json.loads(line))
            if responses[0]!=responses[1]:raise AssertionError(f'{vector["name"]}: Swift {str(responses[0])[:300]} Rust {str(responses[1])[:300]}')
            row=dict(vector)
            if responses[0]=={'kind':'failed_body'}:row['error']=True
            elif 'repeat' not in vector:row['result']=responses[0]
            results.append(row)
            print(vector['name']+': identical',flush=True)
        controls=0
        def control(args,expected):
            responses=[]
            for process in processes:
                process.stdin.write('body '+json.dumps(args)+'\n');process.stdin.flush();responses.append(json.loads(process.stdout.readline()))
            assert responses[0]==responses[1]==expected,(args,responses)
        control({'message_id':'9999'},dict(state='offloaded',plain_text='',html=''));controls+=1
        for args in [{'message_id':'../1'},{'message_id':''},{'message_id':'1','extra':True},{'message_id':'1'*20},{}]:
            control(args,{'kind':'failed_body'});controls+=1
        duplicate=body/'duplicate';duplicate.mkdir();shutil.copyfile(body/'1.emlx',duplicate/'1.emlx')
        subprocess.run(['rsync','-acz',str(root)+'/', 'build:'+remote+'/'],check=True)
        control({'message_id':'1'},{'kind':'failed_body'});controls+=1
        shutil.rmtree(duplicate);hidden=body/'.hidden';hidden.mkdir();shutil.copyfile(body/'1.emlx',hidden/'1.emlx')
        (root/'outside.emlx').write_bytes((body/'1.emlx').read_bytes());(body/'9000.emlx').symlink_to('../outside.emlx')
        (body/'9001.emlx').mkdir();(body/'9002.emlx').symlink_to('1.emlx')
        subprocess.run(['rsync','-acz','--delete',str(root)+'/', 'build:'+remote+'/'],check=True)
        control({'message_id':'1'},results[0]['result']);controls+=1
        for key in ['9000','9001','9002']:
            control({'message_id':key},{'kind':'failed_body'});controls+=1
        print(f'Mail MIME: {controls} matching explicit-path/invalid-argument controls',flush=True)
        hashes=subprocess.check_output(['ssh','build','~/targets/phase6-native/fixture-data-bytes'],input=''.join(raw),text=True).splitlines()
        for index,row in enumerate(results):
            assert hashes[index*2]==hashes[index*2+1],row['name']
            row['data_sha256']=json.loads(hashes[index*2])[0]
        path=pathlib.Path(__file__).with_name('bodies.jsonl');path.write_text(''.join(json.dumps(row,ensure_ascii=False,separators=(',',':'))+'\n' for row in results))
        print(f'Mail MIME: {len(results)} complete fixture results and Go Data byte hashes match')
finally:
    for process in processes:process.stdin.close()
    for process in processes:assert process.wait(timeout=10)==0
    subprocess.run(['ssh','build','rm -rf -- '+shlex.quote(remote)],check=True)
