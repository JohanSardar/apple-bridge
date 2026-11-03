import json
import os
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parents[4]
source = pathlib.Path(__file__).with_name('consent_scenario.txt')
with tempfile.TemporaryDirectory(prefix='ab-gd1-overlay-') as temporary:
    overlay = pathlib.Path(temporary)/'overlay.json'
    overlay.write_text(json.dumps({'Replace': {str(root/'lab/scenarios/gd1_consent_test.go'): str(source)}}))
    environment = dict(os.environ, APPLE_BRIDGE_SECRET_STORE='file', APPLE_BRIDGE_LAB_ROOT=str(root), AB_GO_LAB=str(root/'bin/bridge-lab'), AB_CONSENT_FIXTURE=str(pathlib.Path(sys.argv[1]).resolve()))
    if len(sys.argv)>2:
        environment['AB_CONSENT_EVIDENCE']=str(pathlib.Path(sys.argv[2]).resolve())
    subprocess.run(['go','test','-p','2','-overlay',str(overlay),'./lab/scenarios','-run','^TestRustConsentConnectorProduct$','-count=1','-v'],cwd=root,env=environment,check=True)
