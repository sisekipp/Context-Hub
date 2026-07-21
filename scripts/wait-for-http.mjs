const urls = process.argv.slice(2);
if (!urls.length) throw new Error("Pass at least one URL");
const deadline = Date.now() + Number(process.env.WAIT_TIMEOUT_MS ?? 120_000);
for (const url of urls) {
  while (true) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(3_000) });
      if (response.status < 500) break;
    } catch {}
    if (Date.now() >= deadline) throw new Error(`Timed out waiting for ${url}`);
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  console.log(`${url} is ready`);
}
