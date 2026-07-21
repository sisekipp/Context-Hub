#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
shots_dir="$repo_dir/demo/assets/screenshots"
output_file="${1:-$repo_dir/demo/assets/context-hub-demo-tour.mp4}"
language="${2:-de}"
tour_tmp="$(mktemp -d)"
trap 'rm -rf "$tour_tmp"' EXIT

voice="Anna"
rate="185"

slides=(
  "03-ontology-editor.png|Eine frei definierbare Ontologie|ContextHub modelliert Services, Teams und gemeinsame Interfaces in einem visuellen Editor. Die Beziehungen owned by und depends on sind echte Bestandteile des Schemas und können mit eigenen Informationen versehen werden."
  "10-data-sources.png|Geteilte Datenquellen|Datei-, REST- und GraphQL-Quellen werden zentral im Workspace verwaltet. Jede Ontologie behält trotzdem ihr eigenes Mapping und ihren eigenen isolierten Graphen."
  "04-data-mapping.png|Automatische Schemaerkennung|Die Nova Commerce JSON-Datei enthält einhundertvierundvierzig Services. ContextHub erkennt Felder und Datentypen automatisch und ordnet Identität und Anzeigenamen den Properties der Ontologie zu."
  "05-link-mapping.png|Daten werden zu Beziehungen|Aus owner team entsteht die Beziehung owned by. Aus der Liste depends on entstehen Service-Abhängigkeiten. Das Mapping bleibt deklarativ und wird sicher über DataFusion ausgeführt."
  "06-import-history.png|Nachvollziehbare Importe|Der Import erzeugt einhundertzweiundfünfzig Objekte und fünfhundertsechsundsiebzig Links ohne verworfene Zeile. Historie, Laufzeit, Events, Mapping und Ontologie-Version bleiben vollständig nachvollziehbar."
  "01-explorer-2d-overview.png|Der gesamte Graph in 2D|Der Explorer zeigt die komplette Service-Landschaft mit Teams und Abhängigkeiten. Navigation, Zoom, Typfilter und Labels helfen dabei, auch dichte Graphen zu verstehen."
  "02-explorer-service-inspector.png|Vom Überblick zum Objekt|Eine Suche führt direkt zum Checkout Gateway. Der fokussierte Nachbarschaftsgraph zeigt Eigentümer und Abhängigkeiten, während der Inspector Properties, Links und die Herkunft jedes Feldes erklärt."
  "07-graph-query-builder.png|Graphabfragen ohne Roh-SQL|Im visuellen Query Builder kombinieren Benutzer validierte Filter, Projektionen, Aggregationen und Traversierungen. Hier folgt die Abfrage vom Checkout Gateway zum verantwortlichen Team."
  "09-explorer-3d.png|Die Landschaft in 3D|Dieselben Daten lassen sich interaktiv in drei Dimensionen erkunden, drehen, verschieben und vergrößern. Damit verbindet ContextHub Ontologie, Datenintegration, Provenienz und Graphanalyse in einem System."
)

if [[ "$language" == "en" ]]; then
  voice="Samantha"
  rate="180"
  slides=(
    "03-ontology-editor.png|A freely configurable ontology|ContextHub models services, teams, and shared interfaces in a visual editor. Owned by and depends on are real schema relationships and can carry their own information."
    "10-data-sources.png|Shared data sources|File, REST, and GraphQL sources are managed centrally in the workspace. Each ontology still keeps its own mapping and its own isolated graph."
    "04-data-mapping.png|Automatic schema detection|The Nova Commerce JSON file contains one hundred and forty four services. ContextHub automatically detects fields and data types, then maps identities and display names to ontology properties."
    "05-link-mapping.png|Data becomes relationships|The owner team field creates the owned by relationship. The depends on list creates service dependencies. The mapping stays declarative and runs safely through DataFusion."
    "06-import-history.png|Traceable imports|The import creates one hundred and fifty two objects and five hundred and seventy six links without rejecting a row. History, duration, events, mapping, and ontology version remain fully traceable."
    "01-explorer-2d-overview.png|The complete graph in 2D|The explorer shows the full service landscape with teams and dependencies. Navigation, zoom, type filters, and labels make even dense graphs understandable."
    "02-explorer-service-inspector.png|From overview to object|Search takes us directly to the Checkout Gateway. Its focused neighborhood shows owners and dependencies, while the inspector explains properties, links, and the origin of every field."
    "07-graph-query-builder.png|Graph queries without raw SQL|In the visual Query Builder, users combine validated filters, projections, aggregations, and traversals. This query follows the Checkout Gateway to its responsible team."
    "09-explorer-3d.png|The landscape in 3D|The same data can be explored interactively in three dimensions, with rotation, panning, and zoom. ContextHub brings ontology, data integration, provenance, and graph analysis together in one system."
  )
elif [[ "$language" != "de" ]]; then
  echo "Unsupported language: $language (expected de or en)" >&2
  exit 1
fi

mkdir -p "$(dirname "$output_file")"
: > "$tour_tmp/segments.txt"

for index in "${!slides[@]}"; do
  IFS='|' read -r image_name title narration <<< "${slides[$index]}"
  image_file="$shots_dir/$image_name"
  voice_file="$tour_tmp/voice-$index.aiff"
  segment_file="$tour_tmp/segment-$index.mp4"
  if [[ ! -f "$image_file" ]]; then
    echo "Missing screenshot: $image_file" >&2
    exit 1
  fi

  say -v "$voice" -r "$rate" -o "$voice_file" "$narration"
  voice_duration="$(ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 "$voice_file")"
  segment_duration="$(awk -v duration="$voice_duration" 'BEGIN { printf "%.3f", duration + 1.25 }')"
  fade_out="$(awk -v duration="$segment_duration" 'BEGIN { printf "%.3f", duration - 0.45 }')"

  ffmpeg -hide_banner -loglevel error -y \
    -loop 1 -i "$image_file" -i "$voice_file" \
    -filter_complex "[0:v]scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2:#080c12,format=yuv420p,fade=t=in:st=0:d=0.45,fade=t=out:st=$fade_out:d=0.45[v];[1:a]apad=pad_dur=1.25,afade=t=in:st=0:d=0.2,afade=t=out:st=$fade_out:d=0.4[a]" \
    -map '[v]' -map '[a]' -t "$segment_duration" -r 30 \
    -c:v libx264 -preset medium -crf 18 -c:a aac -b:a 160k -movflags +faststart \
    "$segment_file"
  printf "file '%s'\n" "$segment_file" >> "$tour_tmp/segments.txt"
done

ffmpeg -hide_banner -loglevel error -y -f concat -safe 0 -i "$tour_tmp/segments.txt" -c copy -movflags +faststart "$output_file"
echo "Created $output_file"
