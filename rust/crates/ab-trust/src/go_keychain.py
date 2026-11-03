import ctypes as c,json,sys
r=json.load(sys.stdin)
s=c.CDLL('/System/Library/Frameworks/Security.framework/Security')
p=c.c_void_p
u=c.c_uint32
s.SecKeychainSetUserInteractionAllowed.argtypes=[c.c_bool]
s.SecKeychainOpen.argtypes=[c.c_char_p,c.POINTER(p)]
s.SecKeychainFindGenericPassword.argtypes=[p,u,c.c_char_p,u,c.c_char_p,c.POINTER(u),c.POINTER(p),c.POINTER(p)]
s.SecKeychainAddGenericPassword.argtypes=[p,u,c.c_char_p,u,c.c_char_p,u,c.c_char_p,c.POINTER(p)]
s.SecKeychainItemModifyAttributesAndData.argtypes=[p,p,u,c.c_char_p]
s.SecKeychainItemDelete.argtypes=[p]
s.SecKeychainItemFreeContent.argtypes=[p,p]
code=s.SecKeychainSetUserInteractionAllowed(False)
k=p()
if code==0: code=s.SecKeychainOpen(r['path'].encode(),c.byref(k))
service=r['service'].encode()
account=r['key'].encode()
item=p()
length=u()
data=p()
opened=code==0
if opened: code=s.SecKeychainFindGenericPassword(k,len(service),service,len(account),account,c.byref(length),c.byref(data),c.byref(item))
value=''
if code==0 and r['op']=='get': value=c.string_at(data,length.value).decode()
if data: s.SecKeychainItemFreeContent(None,data)
if opened and r['op']=='put' and code in (0,-25300):
    value=r['value'].encode()
    if code==0: code=s.SecKeychainItemModifyAttributesAndData(item,None,len(value),value)
    else: code=s.SecKeychainAddGenericPassword(k,len(service),service,len(account),account,len(value),value,c.byref(item))
    value=''
if r['op']=='delete' and code==0: code=s.SecKeychainItemDelete(item)
print(json.dumps({'status':code,'value':value}))
