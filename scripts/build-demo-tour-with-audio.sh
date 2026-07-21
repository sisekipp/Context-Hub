#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <audio.wav|audio.mp3|audio.m4a> [output.mp4]" >&2
  exit 1
fi

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
shots_dir="$repo_dir/demo/assets/screenshots"
audio_file="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
output_file="${2:-$repo_dir/demo/assets/context-hub-demo-tour-en-final.mp4}"
tour_tmp="$(mktemp -d)"
trap 'rm -rf "$tour_tmp"' EXIT

slides=(
  "03-ontology-editor.png"
  "10-data-sources.png"
  "04-data-mapping.png"
  "05-link-mapping.png"
  "06-import-history.png"
  "01-explorer-2d-overview.png"
  "02-explorer-service-inspector.png"
  "07-graph-query-builder.png"
  "09-explorer-3d.png"
  "11-mcp-ai-context.png"
)

# Spoken-word weights keep visual chapter changes aligned with a continuous narration.
weights=(31 26 31 31 35 26 31 29 33 45)
weight_total=318
audio_duration="$(ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 "$audio_file")"

mkdir -p "$(dirname "$output_file")"
: > "$tour_tmp/segments.txt"

for index in "${!slides[@]}"; do
  image_file="$shots_dir/${slides[$index]}"
  segment_file="$tour_tmp/segment-$index.mp4"
  segment_duration="$(awk -v duration="$audio_duration" -v weight="${weights[$index]}" -v total="$weight_total" 'BEGIN { printf "%.6f", duration * weight / total }')"
  fade_out="$(awk -v duration="$segment_duration" 'BEGIN { printf "%.6f", duration - 0.35 }')"

  if [[ ! -f "$image_file" ]]; then
    echo "Missing screenshot: $image_file" >&2
    exit 1
  fi

  ffmpeg -hide_banner -loglevel error -y \
    -loop 1 -i "$image_file" \
    -vf "scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2:#080c12,format=yuv420p,fade=t=in:st=0:d=0.35,fade=t=out:st=$fade_out:d=0.35" \
    -t "$segment_duration" -r 30 -an \
    -c:v libx264 -preset medium -crf 18 -movflags +faststart \
    "$segment_file"
  printf "file '%s'\n" "$segment_file" >> "$tour_tmp/segments.txt"
done

ffmpeg -hide_banner -loglevel error -y \
  -f concat -safe 0 -i "$tour_tmp/segments.txt" -i "$audio_file" \
  -map 0:v:0 -map 1:a:0 -af "loudnorm=I=-16:TP=-1.5:LRA=11" \
  -c:v copy -c:a aac -b:a 192k -shortest -movflags +faststart \
  "$output_file"

echo "Created $output_file"
