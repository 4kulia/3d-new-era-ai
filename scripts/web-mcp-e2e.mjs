#!/usr/bin/env node
// The browser as an MCP server, checked the way it is used: a headless tab
// switches its MCP on, an AI client talks to the address the relay hands out,
// and a wall it draws has to be in that tab's project.
//
//   node scripts/web-mcp-e2e.mjs http://127.0.0.1:8801/app/ http://127.0.0.1:7979
//
// Needs the relay running and the site served. Exits non-zero on any step.
import "./web-diagnostics-test.mjs";
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const [page = "http://127.0.0.1:8801/app/", relay = "http://127.0.0.1:7979"] =
  process.argv.slice(2);
const workerSource = readFileSync(new URL("../web/editor/render-worker.js", import.meta.url), "utf8");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const profile = mkdtempSync(join(tmpdir(), "newera-mcp-e2e-"));
const browser = process.env.CHROME
  ?? ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Chromium.app/Contents/MacOS/Chromium"].find((p) => existsSync(p))
  ?? "google-chrome";
const chrome = spawn(browser, [
  "--headless=new", `--user-data-dir=${profile}`,
  ...(process.env.NO_WEBGPU === "1"
    ? ["--disable-features=WebGPU,WebGPUService"]
    : ["--enable-unsafe-webgpu", "--enable-features=Vulkan"]),
  "--use-angle=swiftshader",
  "--window-size=1440,900", "--remote-debugging-port=9555", "about:blank",
], { stdio: "ignore" });

let targets;
for (let i = 0; i < 150 && !targets; i++) {
  await sleep(200);
  targets = await fetch("http://127.0.0.1:9555/json").then((r) => r.json()).catch(() => null);
}
const target = targets?.find((t) => t.type === "page");
if (!target) {
  console.error("Chrome did not open a debuggable page in 30 s");
  process.exit(1);
}
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0;
const pending = new Map();
ws.addEventListener("message", (e) => {
  const msg = JSON.parse(e.data);
  // This disposable test project is intentionally edited and then closed.
  // Once keyboard input has activated the page, its unsaved-work dialog can
  // hold navigation open (and therefore keep the MCP socket alive).
  if (msg.method === "Page.javascriptDialogOpening" && msg.params.type === "beforeunload") {
    void send("Page.handleJavaScriptDialog", { accept: true });
  }
  if (pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
});
const send = (method, params = {}) => new Promise((r) => {
  pending.set(++id, r);
  ws.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression) =>
  (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }))
    .result?.result?.value;
// A hidden tab only draws when a picture is asked of it, and egui only acts
// on a frame: every wait asks for one.
const frames = async (times) => {
  for (let i = 0; i < times; i++) {
    await send("Page.captureScreenshot", { format: "png" });
    await sleep(150);
  }
};

const rpc = async (url, body) => {
  const response = await fetch(url, {
    signal: AbortSignal.timeout(20000),
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
    body: JSON.stringify(body),
  });
  return response.json();
};

let failed = false;
const ok = (what) => console.log(`  ✓ ${what}`);
const bad = (what) => { console.error(`  ✗ ${what}`); failed = true; };

try {
  await send("Page.enable");
  // These canvas hit targets are measured in Portuguese. Linux CI browsers
  // otherwise pick English, which changes both menu widths and line wrapping.
  await send("Page.addScriptToEvaluateOnNewDocument", {source: `
    Object.defineProperty(navigator, 'language', {get: () => 'pt-BR'});
    Object.defineProperty(navigator, 'languages', {get: () => ['pt-BR', 'pt']});
    Object.defineProperty(navigator, 'deviceMemory', {get: () => 2});
    Object.defineProperty(navigator, 'hardwareConcurrency', {get: () => 2});
    const NativeSocket=WebSocket;
    window.WebSocket=class extends NativeSocket {
      constructor(...args) {super(...args); window.testRelaySocket=this;}
    };
  `});
  await send("Page.navigate", { url: `${page}?relay=${encodeURIComponent(relay)}` });
  let started = false;
  for (let i = 0; i < 300 && !started; i++) {
    await sleep(200);
    started = await evaluate("!document.getElementById('loading') && document.readyState === 'complete'");
  }
  if (!started) throw new Error("the editor did not start in 60 s");
  await frames(20);
  if (await evaluate("navigator.language") !== "pt-BR") throw new Error("the UI fixture language was not applied");

  // Nothing is pressed here: a window wide enough to be somebody's desk comes
  // up reachable on its own, which is the whole point — an editor for driving
  // with an AI should not need a switch found first.
  let address = "";
  for (let i = 0; i < 60 && !address; i++) {
    await frames(3);
    address = await evaluate("document.body.getAttribute('data-mcp') || ''");
  }
  if (!address) {
    const log = await evaluate("JSON.stringify((window.neweraLog || []).slice(-6))");
    throw new Error(`the tab never made itself reachable — page said ${log}`);
  }
  ok("the tab came up reachable, with nobody pressing anything");
  const health = await fetch(`${relay}/health`).then((r) => r.json());
  if (health.rooms < 1) bad("the relay has no room for it");

  await check(address);

  async function check(mcpUrl) {
    const hello = await rpc(mcpUrl, {
      jsonrpc: "2.0", id: 1, method: "initialize",
      params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "e2e", version: "1" } },
    });
    if (hello?.result?.serverInfo?.name !== "3d-new-era-ai") bad("the handshake answered wrong");
    else ok("an AI client shook hands with the tab");

    const list = await rpc(mcpUrl, { jsonrpc: "2.0", id: 2, method: "tools/list" });
    const tools = list?.result?.tools ?? [];
    if (tools.length < 30) bad(`the tab offered only ${tools.length} tools`);
    else ok(`the tab offered ${tools.length} tools`);

    await evaluate(`(() => {
      window.diagnosticCalls = [];
      const original = window.neweraOperation;
      window.neweraOperation = (...args) => {
        window.diagnosticCalls.push([args[0], args[2], args[3]]);
        return original(...args);
      };
    })()`);

    // Reproduce drivers that reject mapped-at-creation scene buffers, even
    // for a few MB. A mutation must upload geometry without that crash path.
    await evaluate(`(() => {
      if (!globalThis.GPUDevice) return;
      const original = GPUDevice.prototype.createBuffer;
      GPUDevice.prototype.createBuffer = function(descriptor) {
        if (/^scene (vertices|indices)$/.test(descriptor.label) && descriptor.mappedAtCreation) {
          throw new RangeError('scene buffers cannot be mapped at creation');
        }
        return original.call(this, descriptor);
      };
    })()`);

    const before = await rpc(mcpUrl, {
      jsonrpc: "2.0", id: 3, method: "tools/call",
      params: { name: "home", arguments: {} },
    });
    const made = await rpc(mcpUrl, {
      jsonrpc: "2.0", id: 4, method: "tools/call",
      params: { name: "create", arguments: { walls: [{ pts: [[900, 0], [900, 600]]}] } },
    });
    const said = made?.result?.content?.[0]?.text ?? "";
    if (!said.startsWith("ok")) bad(`the wall did not go in: ${said}`);
    else ok(`the AI drew in the tab: ${said}`);

    const after = await rpc(mcpUrl, {
      jsonrpc: "2.0", id: 5, method: "tools/call",
      params: { name: "home", arguments: {} },
    });
    const beforePlan=JSON.parse(before.result.content[0].text);
    const afterPlan=JSON.parse(after.result.content[0].text);
    const grew = (afterPlan.walls?.length ?? 0) === (beforePlan.walls?.length ?? 0) + 1;
    if (!grew) bad("the project did not change in the tab");
    else ok("the tab's project holds what the AI drew");

    // The ceiling mode is an AI-editable property, independent of visibility.
    const roomId=afterPlan.rooms?.[0]?.id;
    const originalCeilingMode=afterPlan.rooms?.[0]?.ceiling_flat;
    if (!roomId || typeof originalCeilingMode !== 'boolean') throw new Error('ceiling mode is missing from home');
    let ceilingCallId=180;
    const ceilingCall=async(name,args={})=>{
      const reply=await rpc(mcpUrl,{jsonrpc:'2.0',id:ceilingCallId++,method:'tools/call',params:{name,arguments:args}});
      if (reply.error || reply.result?.isError) throw new Error('ceiling MCP call failed: '+JSON.stringify(reply));
      return reply.result.content.find(c=>c.type==='text').text;
    };
    const ceilingHome=async()=>JSON.parse(await ceilingCall('home',{kinds:['rooms']}));
    await ceilingCall('update',{items:[{id:roomId,ceiling_flat:!originalCeilingMode}]});
    const inclined=await ceilingHome();
    if (inclined.rooms.find(r=>r.id===roomId)?.ceiling_flat !== !originalCeilingMode) throw new Error('ceiling mode did not change in the browser');
    await ceilingCall('update',{items:[{id:roomId,ceiling_flat:originalCeilingMode}],dry:true});
    const simulated=await ceilingHome();
    if (simulated.rev !== inclined.rev || simulated.rooms.find(r=>r.id===roomId)?.ceiling_flat !== !originalCeilingMode) throw new Error('ceiling dry-run changed the document');
    await ceilingCall('edit_history',{action:'undo'});
    if ((await ceilingHome()).rooms.find(r=>r.id===roomId)?.ceiling_flat !== originalCeilingMode) throw new Error('undo did not restore the ceiling mode');
    ok('MCP reads and edits ceiling mode in the browser; dry-run and undo preserve the document');

    // Record real worker creation and keep a main-thread heartbeat while photos run.
    await evaluate(`(() => {
      window.renderAudit = { workers: 0, progress: 0, ticks: 0 };
      const Original = window.Worker;
      window.Worker = class extends Original {
        constructor(...args) {
          super(...args); window.renderAudit.workers++; window.lastRenderWorker = this;
          this.addEventListener('message', ({data}) => {
            if (data.type === 'progress') window.renderAudit.progress++;
          });
        }
      };
      window.renderAudit.timer = setInterval(() => window.renderAudit.ticks++, 25);
    })()`);
    // Rendering used to call std::env::temp_dir through the native image
    // cache. On wasm that panics, killing the editor on the first plan PNG.
    for (const name of ["render_plan", "render_3d", "render_photo", "render_plan"]) {
      const rendered = await rpc(mcpUrl, {
        jsonrpc: "2.0", id: 20, method: "tools/call",
        params: { name, arguments: name === "render_photo" ? { w: 64, h: 48 } : { w: 320, h: 240 } },
      });
      const png = rendered?.result?.content?.find((item) => item.type === "image");
      if (rendered?.result?.isError || !png ||
          !Buffer.from(png.data, "base64").subarray(0, 8).equals(Buffer.from([137,80,78,71,13,10,26,10]))) {
        throw new Error(`${name} did not return a PNG`);
      }
      await frames(2);
      const fatal = await evaluate("document.getElementById('failed-why')?.textContent || ''");
      if (fatal) throw new Error(`rendering killed the editor: ${fatal}`);
      ok(`${name} returned a PNG and the editor stayed alive`);
    }
    const audit = await evaluate("JSON.stringify(window.renderAudit)");
    const measured = JSON.parse(audit);
    if (measured.workers !== 4 || measured.progress === 0 || measured.ticks < 4) {
      throw new Error(`renders did not use responsive workers: ${audit}`);
    }
    ok("images ran in disposable workers with progress and a responsive page");
    const oversized = await rpc(mcpUrl, {jsonrpc:'2.0',id:29,method:'tools/call',
      params:{name:'render_plan',arguments:{w:800,h:600}}});
    const reduced = JSON.stringify(oversized);
    if (oversized.result?.isError || !oversized.result?.content?.some(c=>c.type==='image')
        || !reduced.includes('requested 800x600, rendered 640x480') || !reduced.includes('307200')
        || await evaluate('window.renderAudit.workers') !== 5) {
      throw new Error(`small-device budget did not render oversized work at its limit: ${reduced.slice(0, 300)}`);
    }
    ok('small-device capabilities cut an oversized render to the limit and say so');
    // A busy render must leave both ordinary MCP calls and the page usable.
    // Simulate a worker failure to exercise cleanup and the next render.
    const pendingPhoto = rpc(mcpUrl, {jsonrpc:"2.0",id:30,method:"tools/call",
      params:{name:"render_photo",arguments:{w:640,h:480,quality:"best"}}});
    for (let i=0;i<40;i++) {
      if (await evaluate("window.renderAudit.workers > 5")) break;
      await sleep(50);
    }
    const busy = await rpc(mcpUrl, {jsonrpc:"2.0",id:31,method:"tools/call",
      params:{name:"render_plan",arguments:{w:64,h:64}}});
    if (!JSON.stringify(busy).includes("andamento")) throw new Error("a second heavy render was not refused");
    if (process.env.RENDER_SHOT) {
      await frames(3);
      const shot = await send("Page.captureScreenshot", {format:"png"});
      writeFileSync(process.env.RENDER_SHOT, Buffer.from(shot.result.data, "base64"));
    }
    const began = Date.now();
    const during = await rpc(mcpUrl, {jsonrpc:"2.0",id:32,method:"tools/call",params:{name:"home",arguments:{}}});
    if (!during.result || Date.now()-began>3000) throw new Error("MCP blocked behind a render");
    await evaluate("window.lastRenderWorker.dispatchEvent(new ErrorEvent('error',{message:'e2e render failure'}))");
    const stopped = await pendingPhoto;
    if (!JSON.stringify(stopped).includes("e2e render failure")) throw new Error("worker failure was not surfaced");
    const recovered = await rpc(mcpUrl, {jsonrpc:"2.0",id:33,method:"tools/call",
      params:{name:"render_plan",arguments:{w:64,h:64}}});
    if (!recovered.result?.content?.some(c=>c.type==='image')) throw new Error("worker failure leaked the render budget");
    ok("heavy renders are exclusive; the page/MCP remain responsive and worker failure releases the budget");
    const cancelledPhoto = rpc(mcpUrl, {jsonrpc:"2.0",id:34,method:"tools/call",
      params:{name:"render_photo",arguments:{w:640,h:480,quality:"best"}}});
    await frames(4);
    // The render window opens at egui's default (16,16), below its title.
    await send("Input.dispatchMouseEvent", {type:"mouseMoved",x:62,y:113});
    await send("Input.dispatchMouseEvent", {type:"mousePressed",x:62,y:113,button:"left",clickCount:1});
    await send("Input.dispatchMouseEvent", {type:"mouseReleased",x:62,y:113,button:"left",clickCount:1});
    await frames(3);
    const cancelled = await cancelledPhoto;
    if (!JSON.stringify(cancelled).includes("cancelada")) throw new Error("Cancel did not stop the browser render");
    const afterCancel = await rpc(mcpUrl, {jsonrpc:"2.0",id:35,method:"tools/call",
      params:{name:"render_plan",arguments:{w:64,h:64}}});
    if (!afterCancel.result?.content?.some(c=>c.type==='image')) throw new Error("Cancel leaked the render budget");
    ok("the browser Cancel button stops the worker and allows another render");
    // Exercise the same worker/export used by the video window, without a
    // native file dialog. Validate bytes, progress and refusal before allocation.
    const videoAudit = await evaluate(`(async () => {
      const source = ${JSON.stringify(workerSource)};
      const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
      const camera = { x:0, y:0, z:170, yaw:0, pitch:0, fov:63 };
      // Exercise constrained ceiling triangulation in the actual WASM worker.
      const outline = [[0,0,200],[200,0,400],[400,0,200],[400,400,200],[200,400,400],[0,400,200]];
      const walls = outline.map((a,i) => {
        const b=outline[(i+1)%outline.length];
        return {id:'w'+(i+1),start:a.slice(0,2),end:b.slice(0,2),thickness:15,height:a[2],height_at_end:b[2]};
      });
      const rooms=[{id:'r10',name:'Sloped ceiling',points:[[0,0],[400,0],[400,400],[0,400]],ceiling_flat:false}];
      const home = {name:'Worker video', walls, rooms, environment:{
        ground_color:[168,168,152], sky_color:[204,228,252], light_color:[208,208,208], ceiling_light_color:[208,208,208],
        photo:{width:64,height:64}, video:{width:64,frame_rate:2,speed:2}, camera_path:[camera,{...camera,y:100}]
      }};
      const run = (size, render_budget) => new Promise((resolve, reject) => {
        const worker = new Worker(url,{type:'module'});
        let progress=0;
        const timer=setTimeout(() => {worker.terminate();reject(new Error('video timeout'));},15000);
        worker.onmessage=({data})=>{
          if(data.type==='progress') {progress++;return;}
          clearTimeout(timer);worker.terminate();
          resolve({type:data.type,progress,header:data.bytes ? Array.from(data.bytes.slice(0,12)) : [],error:data.error});
        };
        worker.onerror=error=>{clearTimeout(timer);worker.terminate();reject(error);};
        worker.postMessage({module:new URL('./pkg/newera_editor_web.js',location.href).href,
          request:JSON.stringify({kind:'video',home,size,render_budget}),assets:[]});
      });
      const video=await run([64,64]);
      const limit=await run([100000,100000]);
      const restricted=await run([64,64], {pixels:1024,scene_bytes:1048576,asset_bytes:1048576});
      URL.revokeObjectURL(url);
      return {video,limit,restricted};
    })()`);
    if (videoAudit?.video?.type !== 'done' || videoAudit.video.progress < 2 ||
        Buffer.from(videoAudit.video.header).subarray(0,4).toString() !== 'RIFF' ||
        Buffer.from(videoAudit.video.header).subarray(8,12).toString() !== 'AVI ' || videoAudit.limit.type !== 'error'
        || videoAudit.restricted.type !== 'error' || !videoAudit.restricted.error.includes('1024')) {
      throw new Error(`video worker failed: ${JSON.stringify(videoAudit)}`);
    }
    ok("video worker rendered a sloped ceiling in WASM, produced an AVI with progress and rejected excessive resolution");
    // Follow the actual web menu through to a downloaded AVI, catching a
    // disabled menu or a disconnected UI even when the worker itself works.
    // Keep enough viewport height for the menu to open below its button.
    await send("Emulation.setDeviceMetricsOverride", {width:1440,height:900,deviceScaleFactor:1,mobile:false});
    const downloads = mkdtempSync(join(tmpdir(), "newera-video-download-"));
    await send("Browser.setDownloadBehavior", {behavior:"allow",downloadPath:downloads});
    await rpc(mcpUrl, {jsonrpc:"2.0",id:36,method:"tools/call",
      params:{name:"edit_cameras",arguments:{action:"view",i:0}}});
    const click = async (x,y) => {
      await send("Input.dispatchMouseEvent", {type:"mouseMoved",x,y});
      await send("Input.dispatchMouseEvent", {type:"mousePressed",x,y,button:"left",clickCount:1});
      await send("Input.dispatchMouseEvent", {type:"mouseReleased",x,y,button:"left",clickCount:1});
      await frames(2);
    };
    await frames(2);
    await click(169,16); // View menu
    await click(250,137); // Create video
    await click(105,104); // add current visitor camera
    await click(105,104); // repeat: a bounded 0.2-second, five-frame clip
    await click(122,194); // 320 x 240
    await click(88,272); // Generate video
    const movie = join(downloads,"video.avi");
    for (let i=0;i<20 && !existsSync(movie);i++) await frames(1);
    if (!existsSync(movie)) throw new Error("the video UI did not download an AVI");
    const avi = readFileSync(movie);
    if (avi.subarray(0,4).toString() !== "RIFF" || avi.subarray(8,12).toString() !== "AVI ") {
      throw new Error("the video UI downloaded invalid bytes");
    }
    const videoHeader = avi.indexOf('avih') + 8;
    if (videoHeader < 8 || avi.readUInt32LE(videoHeader + 16) !== 5
        || avi.readUInt32LE(videoHeader) !== 40000) {
      throw new Error('the repeated camera points did not produce the predicted five frames at 25 fps');
    }
    ok("Create video opened from the menu and downloaded a valid AVI");
    const undone = await rpc(mcpUrl, {
      jsonrpc: "2.0", id: 21, method: "tools/call",
      params: { name: "edit_history", arguments: {action: "undo"} },
    });
    if (undone?.error || undone?.result?.isError || !undone?.result?.content) {
      throw new Error("editing stopped working after rendering");
    }
    ok("the editor still accepts changes after rendering");

    const diagnosticCalls = JSON.parse(await evaluate('JSON.stringify(window.diagnosticCalls)'));
    if (!diagnosticCalls.some(([event, tool, rev]) => event === 'begin' && tool === 'render_plan' && /^\d+$/.test(rev))
        || !diagnosticCalls.some(([event, tool]) => event === 'end' && tool === 'render_plan')) {
      bad('real MCP calls did not report their diagnostic lifecycle');
    } else ok('real MCP calls report operation and revision before and after execution');

    const readHome = async () => {
      const response = await rpc(mcpUrl, {jsonrpc:"2.0",id:40,method:"tools/call",
        params:{name:"home",arguments:{}}});
      return JSON.parse(response.result.content.find(c=>c.type==='text').text);
    };
    await rpc(mcpUrl,{jsonrpc:"2.0",id:41,method:"tools/call",
      params:{name:"edit_home",arguments:{name:"Recovery confirmed"}}});
    let savedHome;
    for (let i=0;i<30;i++) {
      await frames(2); savedHome=await readHome();
      if (savedHome.recovery.state === 'saved' && savedHome.recovery.saved_revision === savedHome.rev) break;
    }
    if (savedHome.recovery.state !== 'saved' || savedHome.recovery.saved_revision !== savedHome.rev) throw new Error('autosave did not confirm the current revision');
    await evaluate(`(() => {
      window.snapshotBeforeQuota = localStorage.getItem('newera-autosave');
      window.storageSetItem = Storage.prototype.setItem;
      Storage.prototype.setItem = function(key,value) {
        if (key === 'newera-autosave') throw new DOMException('injected quota', 'QuotaExceededError');
        return window.storageSetItem.call(this,key,value);
      };
    })()`);
    await rpc(mcpUrl,{jsonrpc:"2.0",id:42,method:"tools/call",
      params:{name:"edit_home",arguments:{name:"Recovery after quota"}}});
    let failedHome;
    for (let i=0;i<30;i++) {
      await frames(2); failedHome=await readHome();
      if (failedHome.recovery.state === 'failed') break;
    }
    if (failedHome.recovery.state !== 'failed'
        || failedHome.recovery.saved_revision !== savedHome.rev
        || !(await evaluate("localStorage.getItem('newera-autosave') === window.snapshotBeforeQuota"))) {
      throw new Error('storage failure falsely confirmed or replaced the previous snapshot');
    }
    await evaluate('Storage.prototype.setItem = window.storageSetItem');
    const exported = await rpc(mcpUrl,{jsonrpc:"2.0",id:43,method:"tools/call",
      params:{name:"file",arguments:{action:"save",path:"recovery-backup.newera"}}});
    const backupReply=JSON.parse(exported.result.content[0].text);
    const backupPath=join(downloads,'recovery-backup.newera');
    for (let i=0;i<30 && !existsSync(backupPath);i++) await frames(1);
    if (!backupReply.download_started || !existsSync(backupPath)) throw new Error('MCP file save did not download the backup');
    const project=JSON.parse(execFileSync('python3',['-c','import zipfile,sys; print(zipfile.ZipFile(sys.argv[1]).read("project.json").decode())',backupPath],{encoding:'utf8'}));
    if (project.variants[project.active].home.name !== 'Recovery after quota') throw new Error('downloaded backup contains the wrong project');
    const beforeReload=await readHome();
    if (beforeReload.recovery.saved_revision !== beforeReload.rev) throw new Error('explicit backup did not recover from quota failure');
    await evaluate("window.dispatchEvent(new ErrorEvent('error', {message:'RuntimeError: injected recovery failure'}))");
    ok('autosave confirms revision, preserves the previous snapshot on quota failure, and MCP downloads a real project backup');

    // A refresh must not cost the address: the page is reloaded and the same
    // one has to answer again, because it is already pasted into somebody's
    // AI client.
    await send("Page.navigate", { url: `${page}?relay=${encodeURIComponent(relay)}` });
    let backAgain = false;
    for (let i = 0; i < 60 && !backAgain; i++) {
      await frames(3);
      const again = await rpc(mcpUrl, {
        jsonrpc: "2.0", id: 8, method: "tools/call",
        params: { name: "home", arguments: {} },
      });
      backAgain = Boolean(again?.result?.content) && again?.result?.isError !== true;
    }
    if (!backAgain) bad("the address was lost when the page reloaded");
    else ok("reloaded, and the same address still answers");
    const restoredHome=await readHome();
    if (restoredHome.name !== beforeReload.name
        || restoredHome.recovery.restored_from_revision !== beforeReload.rev
        || ['walls','rooms','furniture'].some(key=>JSON.stringify(restoredHome[key]) !== JSON.stringify(beforeReload[key]))) {
      throw new Error('reloading after a runtime failure lost the confirmed snapshot');
    }
    ok('after an injected runtime failure, reload restored the confirmed project geometry and saved revision');
    const legacySnapshotInstalled = await evaluate(`(() => {
      const snapshot=JSON.parse(localStorage.getItem('newera-autosave'));
      const legacy=snapshot.name + '\\n' + snapshot.data;
      localStorage.setItem('newera-autosave', legacy);
      // The current editor may autosave while navigation is still waiting
      // for beforeunload. Keep this fixture intact until its realm is gone;
      // the destination page gets the real Storage prototype again.
      const setItem=Storage.prototype.setItem;
      Storage.prototype.setItem=function(key,value) {
        if (this === localStorage && key === 'newera-autosave') {
          throw new DOMException('Legacy fixture pending reload', 'QuotaExceededError');
        }
        return setItem.call(this,key,value);
      };
      return localStorage.getItem('newera-autosave') === legacy;
    })()`);
    if (!legacySnapshotInstalled) throw new Error('could not install legacy browser snapshot');
    await sleep(2000);
    await send("Page.navigate", {url:`${page}?relay=${encodeURIComponent(relay)}`});
    let legacyHome;
    for (let i=0;i<60;i++) {
      await frames(3);
      try { legacyHome=await readHome(); } catch { continue; }
      if (legacyHome.recovery?.legacy) break;
    }
    if (!legacyHome?.recovery?.legacy || legacyHome.recovery.restored_from_revision !== null || legacyHome.name !== beforeReload.name) {
      throw new Error(`legacy browser snapshot was not restored: ${JSON.stringify(legacyHome?.recovery)}`);
    }
    ok('legacy browser snapshots still restore without pretending their saved revision is known');

    // A mounted but unused 16 MiB import must not exhaust the 8 MiB worker
    // budget. Open a real bundle via the same recovery path just verified.
    const assetBundleFor = (...options) => execFileSync('python3', [
      fileURLToPath(new URL('./fixtures/render-assets.py', import.meta.url)),
      fileURLToPath(new URL('../web/demo.newera', import.meta.url)), ...options,
    ]).toString('base64');
    const loadAssetBundle = async (bundle, name) => {
      await evaluate(`(() => {
        localStorage.setItem('newera-autosave', JSON.stringify({version:1,name:'assets.newera',revision:1,saved_at:Date.now(),data:${JSON.stringify(bundle)}}));
        const setItem=Storage.prototype.setItem;
        Storage.prototype.setItem=function(key,value) {
          if(this===localStorage && key==='newera-autosave') throw new DOMException('Fixture pending reload','QuotaExceededError');
          return setItem.call(this,key,value);
        };
      })()`);
      await send('Page.navigate', {url:`${page}?relay=${encodeURIComponent(relay)}`});
      for(let i=0;i<60;i++) {
        await frames(3);
        let home;
        try {home=await readHome();} catch {continue;}
        if(home.name===name) return home;
      }
      throw new Error('asset bundle did not load: '+name);
    };
    await loadAssetBundle(assetBundleFor(), 'Asset snapshot fixture');
    await evaluate(`(() => {
      const Original=window.Worker;
      window.Worker=class extends Original {
        postMessage(data,transfer) {
          const buffers=data.assets.map(pair=>pair[1].buffer);
          const files=data.assets.map(pair=>pair[0].split('/').pop()).sort();
          super.postMessage(data,transfer);
          window.assetTransfer={files,detached:buffers.every(b=>b.byteLength===0)};
        }
      };
    })()`);
    const assetRender=await rpc(mcpUrl,{jsonrpc:'2.0',id:46,method:'tools/call',
      params:{name:'render_3d',arguments:{w:320,h:240}}});
    const assetTransfer=await evaluate('window.assetTransfer');
    if(assetRender.result?.isError || !assetRender.result?.content?.some(c=>c.type==='image')
        || !assetTransfer?.detached
        || JSON.stringify(assetTransfer.files)!==JSON.stringify(['sample.mtl','sample.obj','wood grain.png'])) {
      throw new Error(`referenced assets were not selected/transferred: ${JSON.stringify(assetTransfer)}`);
    }
    ok('real bundle renders with OBJ/MTL/PNG dependencies; unused imports stay out and buffers are transferred');

    const largeHome=await loadAssetBundle(assetBundleFor('--large-texture'), 'Large texture fixture');
    await evaluate(`(() => {
      window.largeTextureWorkers=0;
      const Original=window.Worker;
      window.Worker=class extends Original {constructor(...args){super(...args);window.largeTextureWorkers++;}};
    })()`);
    const largeRender=await rpc(mcpUrl,{jsonrpc:'2.0',id:47,method:'tools/call',
      params:{name:'render_3d',arguments:{w:320,h:240}}});
    if(!largeRender.result?.isError || !JSON.stringify(largeRender).includes('memória')
        || await evaluate('window.largeTextureWorkers') !== 0) {
      throw new Error('decoded texture budget was not checked before worker creation');
    }
    const largePiece=largeHome.furniture.find(f=>f.model?.endsWith('sample.obj'));
    if(!largePiece) throw new Error('large texture fixture has no model');
    const removed=await rpc(mcpUrl,{jsonrpc:'2.0',id:48,method:'tools/call',
      params:{name:'delete',arguments:{ids:[largePiece.id]}}});
    const retry=await rpc(mcpUrl,{jsonrpc:'2.0',id:49,method:'tools/call',
      params:{name:'render_3d',arguments:{w:320,h:240}}});
    if(removed.result?.isError || retry.result?.isError || !retry.result?.content?.some(c=>c.type==='image')) {
      throw new Error('editor could not recover after refusing excessive decoded texture memory');
    }
    ok('highly compressed texture is refused before pixel allocation/worker creation; editing and retry still work');



    // Two ways an address must die, checked in order of how much they matter.
    //
    // First the switch: Ctrl+Shift+M is the same one the panel's button
    // throws. A loaded runner can be between frames when the keys arrive, so
    // this is only judged when the window says it acted — the page stops
    // publishing its address — and it is the window's word that is checked,
    // not the keyboard's luck.
    const press = async () => {
      for (const [type, extra] of [["keyDown", { text: "" }], ["keyUp", {}]]) {
        await send("Input.dispatchKeyEvent", {
          type, key: "M", code: "KeyM", windowsVirtualKeyCode: 77,
          modifiers: 2 | 8, ...extra,
        });
      }
    };
    let off = false;
    for (let round = 0; round < 3 && !off; round++) {
      await press();
      for (let i = 0; i < 10 && !off; i++) {
        await frames(2);
        off = !(await evaluate("document.body.hasAttribute('data-mcp')"));
      }
    }
    if (off) {
      let closed = false;
      for (let i = 0; i < 20 && !closed; i++) {
        const afterOff = await rpc(mcpUrl, {
          jsonrpc: "2.0", id: 6, method: "tools/call",
          params: { name: "home", arguments: {} },
        });
        closed = afterOff?.result?.isError === true || Boolean(afterOff?.error);
        if (!closed) await sleep(300);
      }
      if (!closed) bad("switched off, and the address still answered");
      else ok("switched off, the address answers nobody");
    } else {
      console.log("  … the window never took the keys; the switch goes unchecked here");
    }

    // Switched on again, the address must be the one already pasted into
    // somebody's AI client: a link that changes is a link that breaks.
    if (off) {
      await press();
      let back = "";
      for (let i = 0; i < 30 && !back; i++) {
        await frames(3);
        back = await evaluate("document.body.getAttribute('data-mcp') || ''");
      }
      if (back !== mcpUrl) bad(`switched on again under a different address: ${back || "none"}`);
      else ok("switched on again, at the very same address");
    }

    // Replace the Rust message handler with a real WASM trap. The emergency
    // responder must answer the in-flight call and future calls without it.
    await evaluate(`(() => {
      const bytes=new Uint8Array([0,97,115,109,1,0,0,0,1,4,1,96,0,0,3,2,1,0,7,8,1,4,116,114,97,112,0,0,10,5,1,3,0,0,11]);
      const trap=new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports.trap;
      window.failedRuntimeCalls=0;
      window.testRelaySocket.onmessage=() => {window.failedRuntimeCalls++; trap();};
    })()`);
    const failureStart=Date.now();
    const trapped=await rpc(mcpUrl,{jsonrpc:'2.0',id:50,method:'tools/call',params:{name:'home',arguments:{}}});
    if (!trapped.result?.isError) throw new Error('a WASM trap did not return an MCP tool error');
    const diagnostic=JSON.parse(trapped.result.content[0].text);
    if (diagnostic.kind !== 'editor_runtime_failure'
        || diagnostic.diagnostic.phase !== 'runtime'
        || diagnostic.diagnostic.operations[0]?.tool !== 'home'
        || !diagnostic.diagnostic.build || diagnostic.diagnostic.backend === 'unknown'
        || !diagnostic.diagnostic.cause.includes('unreachable')
        || Date.now()-failureStart > 10000) throw new Error('the emergency reply lost context or waited for the relay timeout');
    const afterTrap=await rpc(mcpUrl,{jsonrpc:'2.0',id:51,method:'tools/call',params:{name:'home',arguments:{}}});
    if (!afterTrap.result?.isError || await evaluate('window.failedRuntimeCalls') !== 1
        || JSON.stringify(JSON.parse(afterTrap.result.content[0].text)) !== JSON.stringify(diagnostic)) {
      throw new Error('future calls re-entered the failed runtime or lost the original failure');
    }
    ok('a real WASM trap returns structured MCP diagnostics immediately; future calls bypass the failed handler');

    // And then the one that has to hold whatever anybody presses: the tab
    // goes, the address dies. Nothing is left running for an AI to reach.
    // Navigation may retain the document and its socket in the browser's
    // back/forward cache. Close the target to actually exercise tab closure.
    const closeResult=await send("Target.closeTarget", { targetId: target.id });
    if (!closeResult.result?.success) throw new Error("Chrome did not close the target");
    let gone = false;
    for (let i = 0; i < 40 && !gone; i++) {
      const afterGone = await rpc(mcpUrl, {
        jsonrpc: "2.0", id: 7, method: "tools/call",
        params: { name: "home", arguments: {} },
      });
      const disconnectText=afterGone?.result?.content?.[0]?.text ?? '';
      gone = afterGone?.result?.isError === true && /tab (?:is not connected|went away)/.test(disconnectText);
      if (!gone) await sleep(300);
    }
    if (!gone) bad("the tab was closed and its address went on answering");
    else ok("the tab gone, the address answers nobody");
  }
} catch (error) {
  console.error(error.message);
  if (ws.readyState === WebSocket.OPEN) {
    console.error(await evaluate("JSON.stringify((window.neweraLog || []).slice(-12))"));
  }
  failed = true;
} finally {
  ws.close();
  chrome.kill();
}
process.exit(failed ? 1 : 0);
