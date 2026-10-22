import argparse,hashlib,json,subprocess,time,uuid
from pathlib import Path
parser=argparse.ArgumentParser()
parser.add_argument("--app",required=True)
parser.add_argument("--output")
arguments=parser.parse_args()
root=Path(__file__).resolve().parents[2]
run="calmeta_"+time.strftime("%Y%m%dT%H%M%S")+"_"+uuid.uuid4().hex[:8]
tag="[ab-test "+run+"]"
private=Path.home()/"Library/Application Support/AppleBridgeDev/cal-fix"/run
private.mkdir(parents=True,mode=0o700)
app=str(Path(arguments.app).resolve())
def truth(verb,request):
 request=dict(request,source="calendar",tag=tag)
 key=uuid.uuid4().hex
 input=private/(key+"-request.json");output=private/(key+"-report.json")
 input.write_text(json.dumps(request));input.chmod(0o600)
 subprocess.run(["open","-n","-W","-g","-a",app,"--args",verb,"--request-file",str(input),"--report",str(output)],timeout=110,check=True)
 value=json.loads(output.read_text())
 if value.get("status")!="verified":raise RuntimeError(value.get("reason","unverified"))
 return value
def audit():
 return json.loads(subprocess.check_output(["python3",str(root/"lab/truth/calendar-cleanup-audit.py"),"--tag",tag],text=True))
conditions={"load_average":subprocess.check_output(["sysctl","-n","vm.loadavg"],text=True).strip(),"swap_usage":subprocess.check_output(["sysctl","-n","vm.swapusage"],text=True).strip()}
conditions.update({"os_version":subprocess.check_output(["sw_vers","-productVersion"],text=True).strip(),"os_build":subprocess.check_output(["sw_vers","-buildVersion"],text=True).strip(),"sdk_version":subprocess.check_output(["xcrun","--show-sdk-version"],text=True).strip()})
subprocess.run(["codesign","--verify","--strict",app],check=True)
report=Path(app).with_suffix(".cache.json")
if report.exists():
 cache=json.loads(report.read_text())
 conditions.update({"native_source_key":cache["key"],"signed_bundle_sha256":cache["bundle_sha256"]})
conditions["probe_source_sha256"]=hashlib.sha256((root/"lab/truth/native/Sources/AppleTruth/CalendarMetadataProbe.swift").read_bytes()).hexdigest()
result={"run_id":run,"conditions":conditions,"passed":False}
try:
 calendar=truth("fixture",{"action":"create_calendar"})
 result.update(truth("fixture",{"action":"metadata_probe","calendar_id":calendar["id"]}))
 result["passed"]=len(result["fields"])==32 and all((not row["exercised"] and row.get("reason") != "native_probe_error") or (row["exercised"] and row["field_changed"]) for row in result["fields"])
except Exception as error:
 result["error_type"]=type(error).__name__
 print("probe error",type(error).__name__)
finally:
 proof=audit()
 cleanup=truth("fixture",{"action":"cleanup_run_calendar","empty_calendar_proof":proof})
 post=audit()
 result["cleanup"]={"all_dates":post["all_dates"],"remaining":post["remaining"],"owned_calendars_remaining":len(post["counts"]),"removed_calendars":cleanup["removed_calendars"]}
 result["passed"]=result["passed"] and post["remaining"]==0 and not post["counts"]
 output=Path(arguments.output) if arguments.output else root/"lab/evidence/calendar-hints/eventkit-modified-metadata.json"
 output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(result,indent=2)+"\n")
 print(json.dumps({"run_id":run,"passed":result["passed"],"cleanup":result["cleanup"]}))
raise SystemExit(0 if result["passed"] else 1)
