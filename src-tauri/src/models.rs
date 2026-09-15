use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiskMeta {
    pub id: i64,
    pub name: String,
    pub root_path: String,
    pub total_size: i64,
    pub file_count: i64,
    pub created_at: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: i64,
    pub disk_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<i64>,
    pub modified: Option<String>,
    pub path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiskStats {
    pub file_count: i64,
    pub dir_count: i64,
    pub total_size: i64,
    pub thumb_count: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub disk_id: i64,
    pub processed: i64,
    pub current: String,
    pub done: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub node: Node,
    pub disk_name: String,
}
