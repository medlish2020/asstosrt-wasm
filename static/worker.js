window = 'fix stdweb';
importScripts('/asstosrt_wasm.js');

let reader = new FileReaderSync();
let conv_dict = null;

onmessage = async ev => {
  if (ev.data.action == "addFile")
    await addFile(ev.data.id, ev.data.file, ev.data.opts);
  else if (ev.data.action == "addFiles")
    await addFiles(ev.data.id, ev.data.files, ev.data.opts);
  else if (ev.data.action == "preloadDict")
    preloadDict(ev.data.dict);
  else
    throw "unknown action " + ev.data.action;
};

async function addFile(id, file, opts) {
  try {
    opts.conv_dict = await conv_dict;
    const wasm = await Rust.asstosrt_wasm;
    const ass = new Uint8Array(reader.readAsArrayBuffer(file));
    const srt = wasm.assToSrt(ass, opts);
    const url = URL.createObjectURL(srt);
    postMessage({id: id, url: url});
  } catch (e) {
    postMessage({id: id, error: e});
  }
}

async function addFiles(id, files, opts) {
  try {
    opts.conv_dict = await conv_dict;
    let wasm = await Rust.asstosrt_wasm;
    let names = files.map(f => renameToSrt(f.name));
    let contents = files.map(f => new Uint8Array(reader.readAsArrayBuffer(f)));
    let zip = wasm.assToSrtBulk(contents, names, opts);
    let url = URL.createObjectURL(zip);
    postMessage({id: id, url: url});
  } catch (e) {
    postMessage({id: id, error: e});
  }
}

function preloadDict(dict) {
  conv_dict = dict ? fetchChineseConvDict(dict) : null;
}

function renameToSrt(path) {
  if (path.endsWith('.ass'))
    path = path.slice(0, -4);
  return path + ".srt";
}

async function fetchChineseConvDict(dict) {
  if (!dict) return null;
  let resp = await fetch(dict);
  if (!resp.ok) throw "fail to download dict: " + resp.status;
  return await resp.text();
}

