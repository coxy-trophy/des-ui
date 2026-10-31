//! Additional library and playlist operations. Inputs are validated before becoming URLs.
use crate::{protocol::{page_line, problem_line}, web};
use librespot_core::session::Session;
use http::Method;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub enum Feature {
    Browse { kind: String, key: String, offset: u32, cursor: String, range: String },
    Library { uri: String, save: Option<bool> },
    Playlist { id: String, name: String, description: String, public: bool, request_id: u64 },
    PlaylistItem { id: String, uri: String, position: Option<u32>, remove: bool, snapshot: String },
    Reorder { id: String, from: u32, before: u32, snapshot: String },
    PlaylistDetails(String),
    Cover { id: String, jpeg: String, request_id: u64 },
}
fn identifier(text: &str) -> bool { !text.is_empty() && text.len() <= 128 && text.bytes().all(|b|b.is_ascii_alphanumeric()) }
pub fn content_uri(text: &str) -> bool {
    let parts: Vec<_> = text.split(':').collect();
    parts.len()==3 && parts[0]=="spotify" && matches!(parts[1],"track"|"album"|"artist"|"playlist"|"episode"|"show"|"audiobook"|"chapter") && identifier(parts[2])
}
impl Feature {
    pub fn parse(v: &Value) -> Option<Self> {
        let text=|k:&str|v[k].as_str().map(str::to_string);
        let id=||text("id").filter(|s|identifier(s));
        let number=|k:&str|v[k].as_u64().filter(|n|*n<=100_000).map(|n|n as u32);
        Some(match v["cmd"].as_str()? {
            "browse" => {
                let kind=text("kind")?; let key=text("key").unwrap_or_default();
                if !matches!(kind.as_str(),"recent"|"top_tracks"|"top_artists"|"following"|"shows"|"episodes"|"audiobooks"|"show"|"audiobook") {return None;}
                if matches!(kind.as_str(),"show"|"audiobook") && !identifier(&key) {return None;}
                let range=text("range").unwrap_or("medium_term".into());
                if !matches!(range.as_str(),"short_term"|"medium_term"|"long_term") {return None;}
                let cursor=text("cursor").unwrap_or_default();
                if cursor.len()>128 || !cursor.bytes().all(|b|b.is_ascii_alphanumeric()) {return None;}
                Self::Browse{kind,key,offset:number("offset").unwrap_or(0),cursor,range}
            }
            "library" => Self::Library{uri:text("uri").filter(|s|content_uri(s) && !s.starts_with("spotify:chapter:"))?, save:if v["save"].is_null(){None}else{Some(v["save"].as_bool()?) }},
            "playlist_save" => {
                let playlist=text("id").unwrap_or_default(); if !playlist.is_empty() && !identifier(&playlist) {return None;}
                let name=text("name").filter(|s|!s.trim().is_empty() && s.chars().count()<=200)?;
                let description=text("description").unwrap_or_default(); if description.chars().count()>300 {return None;}
                Self::Playlist{id:playlist,name,description,public:v["public"].as_bool()?,request_id:v["request_id"].as_u64().unwrap_or(0)}
            }
            "playlist_item" => {
                let uri=text("uri").filter(|s|content_uri(s) && (s.starts_with("spotify:track:")||s.starts_with("spotify:episode:")))?;
                Self::PlaylistItem{id:id()?,uri,position:if v["position"].is_null(){None}else{Some(number("position")?)},remove:v["remove"].as_bool()?,snapshot:text("snapshot").unwrap_or_default()}
            }
            "playlist_reorder" => Self::Reorder{id:id()?,from:number("from")?,before:number("before")?,snapshot:text("snapshot").filter(|s|!s.is_empty() && s.len()<=256)?},
            "playlist_details" => Self::PlaylistDetails(id()?),
            "playlist_cover" => Self::Cover{id:id()?,jpeg:text("jpeg").filter(|s|!s.is_empty() && s.len()<=256*1024 && s.starts_with("/9j/") && s.bytes().all(|b|b.is_ascii_alphanumeric()||b"+/=".contains(&b)))?,request_id:v["request_id"].as_u64().unwrap_or(0)},
            _ => return None,
        })
    }
}

pub fn media_item(item: &Value, kind: &str) -> Option<Value> {
    let uri=item["uri"].as_str()?;
    let people=item["authors"].as_array().map(|a|a.iter().filter_map(|p|p["name"].as_str()).collect::<Vec<_>>().join(", "));
    Some(json!({"kind":kind,"uri":uri,"id":item["id"],"name":item["name"],
        "artists":item["show"]["name"].as_str().or(item["publisher"].as_str()).map(str::to_string).or(people).unwrap_or_default(),
        "external_url":item["external_urls"]["spotify"],"duration_ms":item["duration_ms"],"cover":web::small_cover(&item["images"]),"album":item["description"]}))
}

fn browse_path(kind:&str,key:&str,offset:u32,cursor:&str,range:&str)->String {
    let page=format!("limit=50&offset={offset}");
    match kind {
        "recent" => format!("/me/player/recently-played?limit=50{}",if cursor.is_empty(){String::new()}else{format!("&before={cursor}")}),
        "top_tracks"|"top_artists" => format!("/me/top/{}?{page}&time_range={range}",if kind=="top_tracks"{"tracks"}else{"artists"}),
        "following" => format!("/me/following?type=artist&limit=50{}",if cursor.is_empty(){String::new()}else{format!("&after={cursor}")}),
        "show" => format!("/shows/{key}/episodes?{page}"),
        "audiobook" => format!("/audiobooks/{key}/chapters?{page}"),
        _ => format!("/me/{kind}?{page}"),
    }
}
pub fn library_path(uri: &str, contains: bool) -> String {
    format!("/me/library{}?uris={}",if contains{"/contains"}else{""},web::encode(uri))
}

impl Feature {
    fn mutation(&self) -> Option<(Method,String,Value)> {
        Some(match self {
            Self::Playlist{id,name,description,public,..}=>(if id.is_empty(){Method::POST}else{Method::PUT},if id.is_empty(){"/me/playlists".into()}else{format!("/playlists/{id}")},json!({"name":name,"description":description,"public":public})),
            Self::PlaylistItem{id,uri,position,remove,snapshot}=>{
                let mut body=if *remove{json!({"items":[{"uri":uri}]})}else{json!({"uris":[uri]})};
                if !remove {if let Some(position)=position{body["position"]=json!(position);}}
                if !snapshot.is_empty(){body["snapshot_id"]=json!(snapshot);}
                (if *remove{Method::DELETE}else{Method::POST},format!("/playlists/{id}/items"),body)
            }
            Self::Reorder{id,from,before,snapshot}=>(Method::PUT,format!("/playlists/{id}/items"),json!({"range_start":from,"insert_before":before,"range_length":1,"snapshot_id":snapshot})),
            _=>return None,
        })
    }
}

async fn browse(session:&Session,kind:&str,key:&str,offset:u32,cursor:&str,range:&str)->String {
    let result=web::send(session,Method::GET,&browse_path(kind,key,offset,cursor,range),None).await;
    let list_key=if kind.starts_with("top_"){range}else{key};
    let found=match result {Ok(v)=>v,Err(e)=>return page_line(kind,list_key,vec![],Some(e),offset,false)};
    let page=if kind=="following"{&found["artists"]}else{&found};
    let entries=page["items"].as_array().cloned().unwrap_or_default();
    let items:Vec<_>=entries.iter().filter_map(|entry|match kind {
        "recent"=>web::track_item(&entry["track"]),"top_tracks"=>web::track_item(entry),
        "top_artists"|"following"=>web::artist_item(entry),
        "shows"=>media_item(&entry["show"],"show"),"episodes"=>media_item(&entry["episode"],"episode"),
        "audiobooks"=>media_item(entry,"audiobook"),"show"=>media_item(entry,"episode"),
        "audiobook"=>media_item(entry,"chapter"),_=>None,
    }).collect();
    let next_cursor=if kind=="recent"{page["cursors"]["before"].as_str()}else{page["cursors"]["after"].as_str()}.unwrap_or_default();
    let more=page["next"].as_str().is_some_and(|s|!s.is_empty()) && !entries.is_empty() && (!matches!(kind,"recent"|"following") || !next_cursor.is_empty() && next_cursor!=cursor);
    format!("{}\n",json!({"type":"list","kind":kind,"key":list_key,"items":items,"error":null,
        "offset":offset,"more":more,"next_offset":offset+entries.len() as u32,"cursor":next_cursor}))
}

/// Success invalidates cached lists; failures never masquerade as completed edits.
fn completed(result: Result<Value,String>) -> String {
    match result {Ok(_)=>format!("{}\n",json!({"type":"changed"})),Err(e)=>problem_line(&e)}
}
pub async fn handle(session:&Session,request:Feature)->String {
    let operation=match &request {
        Feature::Playlist{id,request_id,..}=>Some(format!("playlist_save:{id}:{request_id}")),
        Feature::Cover{id,request_id,..}=>Some(format!("playlist_cover:{id}:{request_id}")),_=>None,
    };
    let line=handle_inner(session,request).await;
    if let Some(operation)=operation {
        if let Ok(mut value)=serde_json::from_str::<Value>(&line){value["operation"]=json!(operation);return format!("{value}\n");}
    }
    line
}

async fn handle_inner(session:&Session,request:Feature)->String {
    if let Some((method,path,body))=request.mutation(){return completed(web::send(session,method,&path,Some(body)).await);}
    match request {
        Feature::Browse{kind,key,offset,cursor,range}=>browse(session,&kind,&key,offset,&cursor,&range).await,
        Feature::Library{uri,save}=>{
            let path=library_path(&uri,save.is_none());
            let result=web::send(session,match save {None=>Method::GET,Some(true)=>Method::PUT,Some(false)=>Method::DELETE},&path,None).await;
            if save.is_some(){completed(result)}else{match result {Ok(v)=>format!("{}\n",json!({"type":"saved","uri":uri,"saved":v[0].as_bool()})),Err(e)=>problem_line(&e)}}
        }
        Feature::Playlist{..}|Feature::PlaylistItem{..}|Feature::Reorder{..}=>unreachable!("handled above"),
        Feature::PlaylistDetails(id)=>match web::send(session,Method::GET,&format!("/playlists/{id}"),None).await {
            Ok(v)=>{
                let user=web::send(session,Method::GET,"/me",None).await.ok();
                let owner=user.as_ref().and_then(|u|u["id"].as_str()).is_some_and(|id|v["owner"]["id"].as_str()==Some(id));
                let editable=v["collaborative"]==true || owner;
                format!("{}\n",json!({"type":"playlist_details","id":id,"name":v["name"],"description":v["description"],"public":v["public"],"snapshot":v["snapshot_id"],"editable":editable,"owner":owner}))
            },Err(e)=>problem_line(&e)},
        Feature::Cover{id,jpeg,..}=>completed(web::send_raw(session,Method::PUT,&format!("/playlists/{id}/images"),"image/jpeg",jpeg.into()).await),
    }
}

#[cfg(test)] mod tests {
 use super::*;
 #[test] fn rejects_paths_and_invalid_mutations(){
  for v in [json!({"cmd":"browse","kind":"../../me"}),json!({"cmd":"browse","kind":"show","key":"../x"}),json!({"cmd":"library","uri":"https://example.org"}),json!({"cmd":"playlist_save","name":"","public":false}),json!({"cmd":"playlist_reorder","id":"abc","from":1,"before":0}),json!({"cmd":"playlist_cover","id":"abc","jpeg":"secret"})] {assert!(Feature::parse(&v).is_none(),"{v}");}
 }
 #[test] fn browse_uses_cursors_and_ranges(){
  assert_eq!(browse_path("recent","",50,"1234","medium_term"),"/me/player/recently-played?limit=50&before=1234");
  assert_eq!(browse_path("following","",50,"abc","medium_term"),"/me/following?type=artist&limit=50&after=abc");
  assert!(browse_path("top_artists","",50,"","long_term").ends_with("offset=50&time_range=long_term"));
 }
 #[test] fn spoken_audio_keeps_its_identity(){let item=media_item(&json!({"uri":"spotify:episode:abc","id":"abc","name":"Episode","show":{"name":"Show"},"duration_ms":9000}),"episode").unwrap();assert_eq!(item["kind"],"episode");assert_eq!(item["artists"],"Show");}
 #[test] fn mutation_contract_preserves_privacy_and_snapshot(){
  let create=Feature::parse(&json!({"cmd":"playlist_save","name":"My playlist","description":"Personal","public":false})).unwrap();
  let (method,path,body)=create.mutation().unwrap();assert_eq!(method,Method::POST);assert_eq!(path,"/me/playlists");assert_eq!(body["public"],false);
  let remove=Feature::parse(&json!({"cmd":"playlist_item","id":"abc","uri":"spotify:episode:def","remove":true,"snapshot":"version1"})).unwrap();
  let (method,path,body)=remove.mutation().unwrap();assert_eq!(method,Method::DELETE);assert_eq!(path,"/playlists/abc/items");assert_eq!(body,json!({"items":[{"uri":"spotify:episode:def"}],"snapshot_id":"version1"}));
  let reorder=Feature::parse(&json!({"cmd":"playlist_reorder","id":"abc","from":0,"before":2,"snapshot":"version1"})).unwrap();
  assert_eq!(reorder.mutation().unwrap().2,json!({"range_start":0,"insert_before":2,"range_length":1,"snapshot_id":"version1"}));
 }
 #[test] fn library_routes_use_uri_based_endpoints(){
  assert_eq!(library_path("spotify:album:abc",false),"/me/library?uris=spotify%3Aalbum%3Aabc");
  assert_eq!(library_path("spotify:track:abc",true),"/me/library/contains?uris=spotify%3Atrack%3Aabc");
  assert_eq!(serde_json::from_str::<Value>(&completed(Err("permission denied".into()))).unwrap()["type"],"problem");
 }

}
