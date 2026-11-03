import json,copy
seeds={
'reminders.create':{'list_id':'rml_fixture','title':'[ab-test vector] <&>'},'reminders.create_list':{'title':'[ab-test vector]'},'reminders.rename_list':{'id':'rml_fixture','title':'x'},'reminders.delete_list':{'id':'rml_fixture'},'reminders.update':{'id':'rem_fixture','priority':0},'reminders.complete':{'id':'rem_fixture'},'reminders.uncomplete':{'id':'rem_fixture'},'reminders.delete':{'id':'rem_fixture'},
'calendar.create_calendar':{'title':'x'},'calendar.create_event':{'calendar_id':'cal_fixture','title':'x','start':'2030-01-01T12:00:00Z','end':'2030-01-01T13:00:00Z','recurrence':[{'frequency':'monthly','interval':1,'count':3}],'alarms':[{'relative_offset':0}]},'calendar.update_event':{'id':'evt_fixture','span':'future','title':'x'},'calendar.delete_event':{'id':'evt_fixture','span':'this'},
'contacts.create':{'given_name':'x','image':None},'contacts.update':{'id':'con_fixture','phones':[{'label':None,'value':'123'}]},'contacts.delete':{'id':'con_fixture'},'contacts.create_group':{'name':'x'},'contacts.rename_group':{'id':'grp_fixture','name':'x'},'contacts.delete_group':{'id':'grp_fixture'},'contacts.add_member':{'id':'grp_fixture','contact_id':'con_fixture'},'contacts.remove_member':{'id':'grp_fixture','contact_id':'con_fixture'},
'notes.create':{'folder_id':'nfo_fixture','title':'x','plain_text':''},'notes.create_folder':{'title':'x'},'notes.update':{'id':'nte_fixture','html':'<p>x</p>'},'notes.rename_folder':{'id':'nfo_fixture','title':'Bridge Test [ab-test vector]'},'notes.move':{'id':'nte_fixture','folder_id':'nfo_fixture'},'notes.delete':{'id':'nte_fixture'},
'messages.send':{'chat_id':'mch_fixture','text':'[ab-test vector] x','files':[{'name':'x.txt','upload_id':'upl_fixture'}]},'messages.reply':{'id':'msg_fixture','part_index':0,'text':'x'},'messages.react':{'id':'msg_fixture','reaction':'emoji','emoji':'👍'},'messages.edit':{'id':'msg_fixture','text':'x'},'messages.unsend':{'id':'msg_fixture'},'messages.mark_read':{'id':'mch_fixture'},'messages.mark_unread':{'id':'mch_fixture'},'messages.delete_message':{'id':'msg_fixture'},'messages.delete_chat':{'id':'mch_fixture'},
'files.create':{'root':'/fixture','name':'x','content_base64':'','digest':'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'},'files.write':{'root':'/fixture','name':'x','content_base64':'','digest':'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'},'files.move':{'root':'/fixture','path':'x','name':'y'},'files.rename':{'root':'/fixture','path':'x','name':'y'},'files.trash':{'root':'/fixture','path':'x'},'files.set_tags':{'root':'/fixture','path':'x','tags':[]},'files.download':{'root':'/fixture','path':'x'},'files.evict':{'root':'/fixture','path':'x'},
'focus.set':{'mode':'Do Not Disturb','on':False},'shortcuts.run':{'shortcut':'fixture','input':'<&>'},'capture.screenshot':{'display':1},'capture.camera_photo':{},'capture.record_audio':{'seconds':1},'mac.screen_record':{'seconds':30},'print.document':{'file':'/fixture/x','printer':'fixture'},'speech.say':{'text':'x'},'capture.play_sound':{'name':'Ping'},
}
for op,args in {'set_volume':{'volume':0},'set_mute':{'muted':False},'set_appearance':{'appearance':'dark'},'launch_app':{'bundle_id':'test.fixture'},'activate_app':{'bundle_id':'test.fixture'},'quit_app':{'bundle_id':'test.fixture'},'keep_awake':{'on':False},'clipboard_write':{'text':''},'notify':{'title':'x'},'open_settings':{'pane':'privacy'},'set_wallpaper':{'image_blob':'eA=='},'iwork_export':{'document_blob':'eA==','format':'pdf','app':'pages'},'lock_screen':{},'sleep_display':{},'restart':{},'shutdown':{}}.items():seeds['mac.'+op]=args
for op in ['ping','capture.photo','location.current','geofence.list','motion.activity','screentime.shield','screentime.unshield','screentime.cancel']:seeds['iphone.'+op]={'device_id':'iphone:fixture'}
for op,args in {'capture.audio':{'seconds':10},'location.significant':{'enabled':False},'geofence.add':{'name':'x','latitude':0,'longitude':0,'radius':1},'geofence.remove':{'name':'x'},'notify':{'title':'x'},'live.start':{'task_id':'x','title':'x','progress':0},'alarm.create':{'fire_at':4102444800},'alarm.update':{'id':'00000000-0000-0000-0000-000000000001','fire_at':4102444800},'alarm.cancel':{'id':'00000000-0000-0000-0000-000000000001'},'alarm.list':{},'timer.start':{'duration_seconds':1},'timer.cancel':{'id':'00000000-0000-0000-0000-000000000001'}}.items():seeds['iphone.'+op]={'device_id':'iphone:fixture',**args}
seeds['iphone.motion.history']={'device_id':'iphone:fixture','days':7}
seeds['iphone.health.log']={'device_id':'iphone:fixture','kind':'water','value':1,'start':'2030-01-01T00:00:00Z','tag':'x'}
seeds['iphone.screentime.schedule']={'device_id':'iphone:fixture','start_minute':1430,'end_minute':10}
inputs=[]
for op,args in seeds.items():
 c={'command_id':'cmd_vector','op':op,'args':args,'expected_version':1 if 'id' in args and not op.startswith('iphone.') else 0}
 ctx={'origin':'owner_self' if op=='messages.send' else '', 'native_key':'native_fixture' if 'id' in args else '', 'files':[{'name':'x.txt','path':'/fixture/x.txt','digest':'abc'}] if op=='messages.send' else []}
 def add(c):inputs.append({'command':copy.deepcopy(c),'context':ctx})
 add(c)
 for version in [0,1]:d=copy.deepcopy(c);d['expected_version']=version;add(d)
 for id in ['', 'x/y','a'*129,'cmd:-._0']:d=copy.deepcopy(c);d['command_id']=id;add(d)
 for wait in [-1,15000,15001]:d=copy.deepcopy(c);d['wait_ms']=wait;add(d)
 for key in args:
  for alias in [key.upper(), ''.join(x.title() for x in key.split('_'))]:
   d=copy.deepcopy(c);d['args'][alias]=d['args'].pop(key);add(d)
  for value in [None,False,0,'',[],{},'\u2003',-1]:d=copy.deepcopy(c);d['args'][key]=value;add(d)
  d=copy.deepcopy(c);del d['args'][key];add(d)
 for key in ['unknown','parts','variant']:
  d=copy.deepcopy(c);d['args'][key]=[];add(d)
print(json.dumps(inputs))
