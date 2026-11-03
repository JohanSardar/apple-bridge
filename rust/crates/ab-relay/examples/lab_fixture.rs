use ab_relay::{
    Error, Result,
    http::{Api, DnsProvider, NameResolver},
    server::Server,
    store::{Enrollment, Limits, Proof, Store},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, State},
    http::Request,
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

#[derive(Debug)]
struct Callback {
    endpoint: String,
    client: reqwest::Client,
}
impl DnsProvider for Callback {
    fn txt<'a>(
        &'a self,
        name: &'a str,
        value: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.client
                .post(format!("{}/dns", self.endpoint))
                .json(&json!({"name":name,"value":value}))
                .send()
                .await
                .map_err(|_| Error::Denied)?
                .error_for_status()
                .map_err(|_| Error::Denied)?;
            Ok(())
        })
    }
}
impl NameResolver for Callback {
    fn cname<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            self.client
                .get(format!("{}/cname", self.endpoint))
                .query(&[("name", name)])
                .send()
                .await
                .map_err(|_| Error::Denied)?
                .error_for_status()
                .map_err(|_| Error::Denied)?
                .json()
                .await
                .map_err(|_| Error::Denied)
        })
    }
}
#[derive(Debug)]
struct NotifyClose {
    endpoint: String,
    key: String,
}
impl Drop for NotifyClose {
    fn drop(&mut self) {
        let endpoint = self.endpoint.clone();
        let key = self.key.clone();
        tokio::spawn(async move {
            let _ = reqwest::Client::new()
                .post(format!("{endpoint}/cancel"))
                .json(&json!({"key":key}))
                .send()
                .await;
        });
    }
}
#[derive(Debug)]
struct Fixture {
    store: Arc<Mutex<Store>>,
    servers: Mutex<HashMap<u64, Arc<Api>>>,
    listeners: Mutex<HashMap<u64, CancellationToken>>,
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().ok_or(Error::Denied)
}
fn number(v: &Value, key: &str) -> Result<u64> {
    v[key].as_u64().ok_or(Error::Denied)
}
impl Fixture {
    async fn command(&self, v: Value) -> Result<Value> {
        let action = field(&v, "action")?;
        match action {
            "owner" => {
                self.store
                    .lock()
                    .await
                    .require_owner_keys(field(&v, "path")?.into());
                Ok(Value::Null)
            }
            "validate" => {
                self.store.lock().await.validate_owner_keys()?;
                Ok(Value::Null)
            }
            "device" => Ok(serde_json::to_value(
                self.store.lock().await.devices.get(field(&v, "host")?),
            )?),
            "policy" => {
                self.store.lock().await.policy(
                    field(&v, "host")?,
                    serde_json::from_value(v["limits"].clone())?,
                    v["disabled"].as_bool().unwrap_or(false),
                    v["revoked"].as_bool().unwrap_or(false),
                )?;
                Ok(Value::Null)
            }
            "verify" => Ok(json!(self.store.lock().await.verify(
                &serde_json::from_value::<Proof>(v["proof"].clone())?,
                field(&v, "proof_action")?
            ))),
            "challenge" => Ok(json!(self.store.lock().await.challenge()?)),
            "enroll" => {
                Ok(serde_json::to_value(self.store.lock().await.enroll(
                    serde_json::from_value::<Enrollment>(v["input"].clone())?,
                )?)?)
            }
            "new" => {
                let callback = Arc::new(Callback {
                    endpoint: field(&v, "callback")?.to_owned(),
                    client: reqwest::Client::new(),
                });
                let server = Arc::new(Server {
                    store: self.store.clone(),
                    sessions: Mutex::new(HashMap::new()),
                    idle: Duration::from_secs(2),
                    control: None,
                });
                let mut api = Api::new(server, v["admin"].as_str().unwrap_or("").to_owned())?;
                api.dns = Some(callback.clone());
                api.resolver = callback.clone();
                api.ct_origin = format!("{}/ct", callback.endpoint);
                let mut servers = self.servers.lock().await;
                let id = u64::try_from(servers.len()).map_err(|_| Error::Denied)? + 1;
                servers.insert(id, Arc::new(api));
                Ok(json!(id))
            }
            _ => self.server_command(v).await,
        }
    }
    async fn server_command(&self, v: Value) -> Result<Value> {
        let id = number(&v, "id")?;
        let api = self
            .servers
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or(Error::Denied)?;
        match field(&v, "action")? {
            "control" => {
                let options = ab_relay::acme::Options {
                    directory: field(&v, "directory")?.to_owned(),
                    root: Some(field(&v, "root")?.into()),
                };
                let _certificates = ab_relay::acme::start_with_options(
                    field(&v, "path")?.into(),
                    field(&v, "host")?.to_owned(),
                    api.dns.clone().ok_or(Error::Denied)?,
                    CancellationToken::new(),
                    options,
                )
                .await?;
                Ok(Value::Null)
            }

            "handler" => {
                let mut handler = Api::new(api.server.clone(), field(&v, "admin")?.to_owned())?;
                handler.dns = api.dns.clone();
                handler.resolver = api.resolver.clone();
                handler.ct_origin.clone_from(&api.ct_origin);
                handler.warnings = api.warnings.clone();
                let mut servers = self.servers.lock().await;
                let next = u64::try_from(servers.len()).map_err(|_| Error::Denied)? + 1;
                servers.insert(next, Arc::new(handler));
                Ok(json!(next))
            }

            "disconnect" => {
                api.server.disconnect(field(&v, "host")?).await;
                Ok(Value::Null)
            }
            "close" => {
                for s in api.server.sessions.lock().await.drain().map(|(_, s)| s) {
                    s.cancel.cancel();
                }
                Ok(Value::Null)
            }
            "stop" => {
                if let Some(stop) = self.listeners.lock().await.remove(&id) {
                    stop.cancel();
                }
                Ok(Value::Null)
            }
            "listen" => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let address = listener.local_addr()?;
                let cancel = CancellationToken::new();
                self.listeners.lock().await.insert(id, cancel.clone());
                tokio::spawn(async move {
                    let _ = api.server.clone().serve(listener, cancel).await;
                });
                Ok(json!(address.to_string()))
            }
            "monitor" => {
                ab_relay::ct::monitor_once(&api, &reqwest::Client::new()).await;
                Ok(Value::Null)
            }
            "attach" => {
                let (tx, mut rx) = tokio::sync::mpsc::channel::<ab_relay::mux::Open>(32);
                let cancel = CancellationToken::new();
                let session = api
                    .server
                    .attach(field(&v, "host")?, field(&v, "token")?, tx, cancel.clone())
                    .await?;
                let address = field(&v, "address")?.to_owned();
                let callback = field(&v, "callback")?.to_owned();
                tokio::spawn(async move {
                    let mut tasks = tokio::task::JoinSet::new();
                    loop {
                        tokio::select! {
                         ()=cancel.cancelled()=>break,
                         Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
                         open=rx.recv()=>{let Some(open)=open else{break};let address=address.clone();let callback=callback.clone();tasks.spawn(async move{
                          if let Ok(mut socket)=TcpStream::connect(address).await{let _notify=NotifyClose { endpoint:callback,key:socket.local_addr().map(|a|a.to_string()).unwrap_or_default() };let(local,remote)=tokio::io::duplex(262_144);if open.reply.send(local).is_ok(){let(mut sr,mut sw)=socket.split();let(mut rr,mut rw)=tokio::io::split(remote);tokio::select! { _=tokio::io::copy(&mut sr,&mut rw)=>{},_=tokio::io::copy(&mut rr,&mut sw)=>{} }}}
                         });}
                        }
                    }
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                });
                Ok(json!(session.token))
            }
            _ => Err(Error::Denied),
        }
    }
}
async fn rpc(State(fixture): State<Arc<Fixture>>, Json(v): Json<Value>) -> Json<Value> {
    match fixture.command(v).await {
        Ok(value) => Json(json!({"ok":true,"value":value})),
        Err(e) => Json(json!({"ok":false,"error":e.to_string()})),
    }
}
async fn api(
    State(fixture): State<Arc<Fixture>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    mut request: Request<Body>,
) -> Response {
    let path = request.uri().path().to_owned();
    let Some(rest) = path.strip_prefix("/api/") else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Some((id, path)) = rest.split_once('/') else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Ok(id) = id.parse::<u64>() else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Some(api) = fixture.servers.lock().await.get(&id).cloned() else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let query = request
        .uri()
        .query()
        .map_or(String::new(), |q| format!("?{q}"));
    let Ok(uri) = format!("/{path}{query}").parse() else {
        return axum::http::StatusCode::BAD_REQUEST.into_response();
    };
    *request.uri_mut() = uri;
    request.extensions_mut().insert(ConnectInfo(peer));
    match api.router().oneshot(request).await {
        Ok(response) => response,
        Err(e) => match e {},
    }
}
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("fixture requires state zone limits-json".into());
    }
    let limits: Limits = serde_json::from_str(&args[2])?;
    let fixture = Arc::new(Fixture {
        store: Arc::new(Mutex::new(Store::open(
            args[0].clone().into(),
            &args[1],
            limits,
        )?)),
        servers: Mutex::new(HashMap::new()),
        listeners: Mutex::new(HashMap::new()),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    println!("http://{}", listener.local_addr()?);
    let router = Router::new()
        .route("/rpc", post(rpc))
        .fallback(api)
        .with_state(fixture);
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
