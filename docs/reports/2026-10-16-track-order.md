# Track swap + video-above-audio — done

1. Sections: Video (top) → Audio → Sfx; `create_project` / `add_track` / `ensure_av_tracks` maintain this.
2. `MoveTrack { track_id, up }` swaps adjacent same-kind neighbors only.
3. UI: track context menu 上移/下移 (disabled at kind boundaries).
4. Undo restores order; paste/place no longer assume `tracks.last()`.
5. Tests: `track_order`, insertion, interaction gesture, related editor_basics — green.
