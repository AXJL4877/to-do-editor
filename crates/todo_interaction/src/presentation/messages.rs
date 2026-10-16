use todo_application::Status;

/// Localized display belongs to presentation; core and application carry typed status.
pub fn status_text(status: &Status) -> String {
    match status {
        Status::Domain(error) => match error.message_key() {
            "media_not_found" => "素材不存在",
            "folder_not_found" => "文件夹不存在",
            "folder_cycle" => "不能将文件夹移入自身或其子文件夹",
            "track_not_found" => "目标轨道不存在",
            "clip_not_found" => "片段不存在",
            "track_locked" => "轨道已锁定",
            "incompatible_track" => "素材与目标轨道类型不兼容",
            "overlap" => "操作会与相邻片段重叠",
            "source_bounds" => "已达到源素材的入出点边界",
            "clip_too_short" => "片段至少保留一帧",
            "invalid_time" | "invalid_duration" => "时间或时长无效",
            "invalid_output_size" => "画布分辨率无效",
            "invalid_fps" => "帧率无效",
            "invalid_transform" => "画面位置、尺寸、缩放或透明度无效",
            "track_required" => "至少保留一个视频轨和一个音频轨",
            "linked_edit_mismatch" => "关联片段无法同步完成该操作",
            "ripple_conflict" => "波纹删除会影响受保护的片段",
            "media_in_use" => "素材仍在时间轴中使用",
            "track_order" => "已在同类型轨道的边界，无法继续移动",
            _ => "工程数据或编辑参数无效",
        }
        .to_owned(),
        Status::External { operation, message } => format!(
            "{}：{message}",
            match *operation {
                "save" => "保存失败",
                "open" => "打开工程失败",
                "list" => "读取工程列表失败",
                "delete" => "删除工程失败",
                "import" => "导入失败",
                _ => "操作失败",
            }
        ),
        Status::Copied => "已复制所选片段".into(),
        Status::WaitingImport => "请等待当前导入完成".into(),
        Status::Saving => "正在保存工程…".into(),
    }
}
