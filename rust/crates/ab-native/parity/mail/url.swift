import Foundation
for value in ["imap://user@example.invalid/INBOX","imap://user%40test:password@EXAMPLE.invalid/My%20Folder/","imap://EXAMPLE.invalid/A%2FB","imap://[2001:db8::1]:993/Inbox","imap://@example.invalid/INBOX","imap://user@example.invalid","imap://user@example.invalid/","imap://user@example.invalid/a/../Sent","imap://user@example.invalid/Café","imap://user@example.invalid/a?x#y","file://localhost/Synthetic","invalid","https://EXAMPLE.invalid/Inbox","imap://us%3Aer@exa%6Dple.invalid/Inbox"] {
    let u=URL(string:value)
    let out:[String:Any]=["input":value,"valid":u != nil,"scheme":u?.scheme as Any? ?? NSNull(),"host":u?.host as Any? ?? NSNull(),"user":u?.user as Any? ?? NSNull(),"path":u?.path as Any? ?? NSNull(),"last":u?.lastPathComponent as Any? ?? NSNull()]
    let data=try JSONSerialization.data(withJSONObject:out,options:[.sortedKeys,.withoutEscapingSlashes]);print(String(decoding:data,as:UTF8.self))
}

for value in ["imap://host///","imap://host/a//","imap://host//a//b/","//Host/Inbox","imap:///INBOX","imap://bad host/Inbox","imap://host:broken/Inbox","imap://[broken]/Inbox","imap://例.invalid/收件箱","imap://ÉXAMPLE.invalid/Inbox","imap://user@host/%FF","imap://user@host/%zz","imap://us%00er@host/Inbox","imap://host/a%2F","imap://host/./Inbox","imap://host/..","imap://host/a#fragment","imap://user@@host/Inbox"] {
    let u=URL(string:value)
    let out:[String:Any]=["input":value,"valid":u != nil,"scheme":u?.scheme as Any? ?? NSNull(),"host":u?.host as Any? ?? NSNull(),"user":u?.user as Any? ?? NSNull(),"path":u?.path as Any? ?? NSNull(),"last":u?.lastPathComponent as Any? ?? NSNull()]
    let data=try JSONSerialization.data(withJSONObject:out,options:[.sortedKeys,.withoutEscapingSlashes]);print(String(decoding:data,as:UTF8.self))
}
