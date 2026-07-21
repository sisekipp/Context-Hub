# ContextHub demo tour

The demo follows a fictional commerce platform called **Nova Commerce**:

1. Define `Service`, `Team`, and `Deployable` in the ontology editor.
2. Manage the shared JSON source in the workspace catalog.
3. Map source fields to ontology properties.
4. Create `owned_by` and `depends_on` relationships from source references.
5. Verify the DataFusion ingestion and its provenance.
6. Explore all 152 objects and 576 relationships in 2D.
7. Search for Checkout Gateway and inspect its neighborhood.
8. Build a bounded traversal with the visual Graph Query Builder.
9. Rotate and zoom the same graph in 3D.

Regenerate the assets with:

```bash
node scripts/generate-demo-data.mjs
bash scripts/build-demo-tour.sh
bash scripts/build-demo-tour.sh demo/assets/context-hub-demo-tour-en.mp4 en
bash scripts/build-demo-tour-with-audio.sh /path/to/voice-over.wav
```

The video builder supports German (`Anna`) and English (`Samantha`) narration and
requires the macOS `say` command, `ffmpeg`, and `ffprobe`. The external-audio
builder normalizes a supplied voice-over and includes the MCP/AI closing chapter.
