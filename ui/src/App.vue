<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderSync, Network, Monitor, Files, Upload, Download, RefreshCw, Plus, ArrowRight, Settings, LogOut, X } from '@lucide/vue';
import { api, desktop, setToken, size, stateNames } from './api';

type Row = Record<string,any>;
const ready=ref(false), busy=ref(false), error=ref(''), notice=ref(''), token=ref('');
const page=ref(desktop?'receive':'nodes'), search=ref(''), offset=ref(0), total=ref(0), filter=ref('attention');
const nodes=ref<Row[]>([]), edges=ref<Row[]>([]), rows=ref<Row[]>([]), sources=ref<Row[]>([]), maps=ref<Row[]>([]);
const info=ref<Row>({}), status=ref<Row>({}), relation=ref<Row>({upstreams:[],downstreams:[]});
const modal=ref(''), credential=ref<Row|null>(null), selected=ref<Row|null>(null);
const name=ref(''), up=ref(''), down=ref(''), prefix=ref(''), local=ref(''), from=ref(''), batch=ref(false);
const hub=ref('http://127.0.0.1:8787'), nodeId=ref(''), nodeToken=ref(''), ca=ref('');
const currentNodeName=computed(()=>nodes.value.find(n=>n.id===info.value.node_id)?.name ?? info.value.name ?? info.value.node_id ?? '本机节点');
const nav=computed(()=>{
  if(!desktop)return [{id:'nodes',label:'节点管理',icon:Monitor},{id:'edges',label:'分发关系',icon:Network},{id:'deliveries',label:'分发结果',icon:Files}];
  if(relation.value.downstreams.length && !relation.value.upstreams.length)return [{id:'publish',label:'发布文件',icon:Upload},{id:'deliveries',label:'分发结果',icon:Files}];
  if(relation.value.upstreams.length && !relation.value.downstreams.length)return [{id:'receive',label:'接收文件',icon:Download},{id:'maps',label:'保存位置',icon:Settings}];
  return [{id:'connection',label:'节点状态',icon:Monitor}];
});
const title=computed(()=>nav.value.find(n=>n.id===page.value)?.label ?? 'FileSync');
const nodeName=(id:string)=>nodes.value.find(n=>n.id===id)?.name ?? [...relation.value.upstreams,...relation.value.downstreams].find(n=>n.id===id)?.name ?? id;
let generation=0, timer:ReturnType<typeof setInterval>|undefined;
async function action(fn:()=>Promise<void>) { if(busy.value)return; busy.value=true; error.value=''; notice.value=''; try{await fn()}catch(e){error.value=String(e instanceof Error?e.message:e)}finally{busy.value=false} }
async function load() {
  const current=++generation;
  const q=`offset=${offset.value}&limit=50&search=${encodeURIComponent(search.value)}`;
  if(desktop){
    const [s,i,r]=await Promise.all([api('/status'),api('/info'),api('/relations').catch(()=>relation.value)]);
    if(current!==generation)return;
    status.value=s;info.value={...info.value,...i};relation.value=r;
    if(!nav.value.some(item=>item.id===page.value)){page.value=nav.value[0]!.id;offset.value=0;rows.value=[];}
  } else {
    const [n,e]=await Promise.all([api('/v1/admin/nodes'),api('/v1/admin/edges')]);
    if(current!==generation)return; nodes.value=n;edges.value=e;
  }
  if(page.value==='receive') {const p=await api(`/files?${q}&state=${filter.value}`);if(current===generation){rows.value=p.items;total.value=p.total}}
  if(page.value==='deliveries') {const p=await api(`${desktop?'/deliveries':'/v1/admin/deliveries'}?${q}`);if(current===generation){rows.value=p.items;total.value=p.total}}
  if(page.value==='publish'){const p=await api('/sources');if(current===generation)sources.value=p;}
  if(page.value==='maps'){const p=await api('/maps');if(current===generation)maps.value=p;}
}
async function login(){await action(async()=>{setToken(token.value.trim());await load();ready.value=true;token.value='';})}
async function connectExisting(configPath?:string){await action(async()=>{info.value=await invoke('desktop_connect',{configPath:configPath??null});await load();ready.value=true;})}
async function setup(){await action(async()=>{info.value=await invoke('desktop_setup',{input:{hub:hub.value.trim(),node_id:nodeId.value.trim(),token:nodeToken.value.trim(),ca_cert:ca.value||null}});nodeToken.value='';await load();ready.value=true;})}
async function importConfig(){const file=await open({multiple:false,filters:[{name:'TOML',extensions:['toml']}]});if(typeof file==='string')await connectExisting(file);}
async function chooseCa(){const file=await open({multiple:false,filters:[{name:'CA certificate',extensions:['pem','crt']}]});if(typeof file==='string')ca.value=file;}
async function chooseSource(directory:boolean){const path=await open({directory,multiple:false});if(typeof path==='string'){local.value=path;prefix.value=path.split(/[\\/]/).pop()??'';modal.value='source';}}
async function chooseFolder(){const path=await open({directory:true,multiple:false});if(typeof path==='string')local.value=path;}
function startClaim(row:Row,isBatch:boolean){selected.value=row;batch.value=isBatch;from.value=isBatch?row.path.slice(0,row.path.lastIndexOf('/')+1):row.path;local.value='';modal.value='claim';}
async function saveClaim(){await action(async()=>{const row=selected.value!;const target=batch.value?local.value:`${local.value.replace(/[\\/]$/,'')}/${row.path.split('/').pop()}`;await api('/claim','POST',{upstream:row.upstream,from:from.value,to:target});modal.value='';notice.value='保存位置已设置，后台将自动接收文件。';await load();})}
async function saveSource(){await action(async()=>{await api('/sources','POST',{prefix:prefix.value.trim(),path:local.value});modal.value='';notice.value='已加入发布目录，后台开始扫描。';await load();})}
async function newNode(){await action(async()=>{credential.value=await api('/v1/nodes','POST',{name:name.value.trim()});modal.value='credential';name.value='';await load();})}
async function saveEdge(){await action(async()=>{await api('/v1/edges','POST',{upstream:up.value,downstream:down.value});modal.value='';await load();})}
function askRemove(kind:string,row:Row){selected.value=row;modal.value=kind;}
async function remove(){await action(async()=>{if(modal.value==='removeNode'){await api('/v1/admin/nodes/'+encodeURIComponent(selected.value!.id),'DELETE');modal.value='';notice.value='节点已删除，原令牌已失效。';await load();return;}const endpoints:Record<string,string>={removeEdge:'/v1/admin/edges',removeSource:'/sources',removeMap:'/maps'};const s=selected.value!;const body=modal.value==='removeSource'?{path:s.path,prefix:s.prefix}:modal.value==='removeMap'?{upstream:s.upstream,from:s.from,to:s.to}:s;await api(endpoints[modal.value]!,'DELETE',body);modal.value='';await load();})}
function saveCredential(){if(!credential.value)return;const blob=new Blob([JSON.stringify({...credential.value,hub:location.origin},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob);const a=document.createElement('a');a.href=url;a.download='filesync-node-credential.json';a.click();URL.revokeObjectURL(url);}
function dismiss(){if(busy.value)return;modal.value='';credential.value=null;}
function logout(){generation++;setToken('');ready.value=false;nodes.value=[];edges.value=[];rows.value=[];credential.value=null;modal.value='';}
watch(page,()=>{offset.value=0;rows.value=[];search.value='';if(ready.value)void action(load)});
onMounted(async()=>{if(desktop) { const exists=await invoke<boolean>('desktop_has_config').catch(()=>false);if(exists)await connectExisting(); } timer=setInterval(()=>{if(ready.value&&!busy.value&&!modal.value)void action(load)},10000)});
onUnmounted(()=>clearInterval(timer));
</script>

<template>
  <div v-if="!ready" class="welcome">
    <div class="welcome-story"><div class="brand"><FolderSync :size="30"/> FileSync</div><p class="eyebrow">团队文件分发</p><h1>文件到位，<br>工作继续。</h1><p>上游发布，下游选择保存位置。<br>之后的更新，交给后台完成。</p><div class="flow"><span>上游</span><ArrowRight/><span>中心</span><ArrowRight/><span>下游</span></div><small>两层分发 · 最新版本 · 覆盖前保留 3 份备份</small></div>
    <main class="welcome-form">
      <p class="eyebrow">{{desktop?'节点桌面客户端':'中心管理控制台'}}</p><h2>{{desktop?'连接这台电脑':'登录中心'}}</h2>
      <p class="muted">{{desktop?'填写中心创建的节点 ID 和令牌，完成首次连接。':'使用启动中心时设置的管理员令牌。'}}</p>
      <form v-if="!desktop" @submit.prevent="login"><label>管理员令牌<input v-model="token" type="password" required minlength="32" autocomplete="off" placeholder="输入管理员令牌"/></label><button class="primary wide" :disabled="busy">{{busy?'正在连接…':'进入管理台'}}</button><small>令牌只保留在当前页面内存中，刷新后需重新登录。</small></form>
      <form v-else @submit.prevent="setup"><label>中心地址<input v-model="hub" required type="url"/></label><label>节点 ID<input v-model="nodeId" required placeholder="从中心节点管理页面获取"/></label><label>节点令牌<input v-model="nodeToken" type="password" required minlength="32" autocomplete="off"/></label><label>自签证书的 CA（可选）<div class="inline"><input v-model="ca" readonly placeholder="使用公开可信证书时无需选择"/><button type="button" @click="chooseCa">选择</button></div></label><button class="primary wide" :disabled="busy">{{busy?'正在连接…':'连接并启动同步'}}</button><button class="wide text-button" type="button" :disabled="busy" @click="importConfig">已有配置？选择 TOML 文件</button></form>
      <p v-if="error" role="alert" class="alert">{{error}}</p>
    </main>
  </div>
  <div v-else class="shell">
    <aside><div class="brand"><FolderSync :size="27"/> FileSync</div><div class="edition">{{desktop?'桌面客户端':'中心管理'}}</div><nav><button v-for="item in nav" :key="item.id" :class="{active:page===item.id}" @click="page=item.id"><component :is="item.icon" :size="19"/>{{item.label}}</button></nav><div class="aside-foot"><span class="dot"></span>{{desktop?'后台独立运行':'中心已连接'}}<small>{{desktop?'关闭窗口后继续同步':'两层文件分发'}}</small></div></aside>
    <div class="workspace"><header><span>{{desktop?currentNodeName:'团队工作空间'}}</span><div class="inline"><span v-if="desktop" class="muted">{{stateNames[status.state]??status.state}}</span><button :disabled="busy" @click="action(load)" aria-label="刷新"><RefreshCw :size="16"/>刷新</button><button v-if="!desktop" @click="logout"><LogOut :size="16"/>退出登录</button></div></header>
    <main class="content"><div class="page-heading"><div><p class="eyebrow">{{desktop?'我的文件工作台':'管理工作台'}}</p><h1>{{title}}</h1><p class="muted">{{page==='nodes'?'先创建节点，再配置谁向谁分发文件。':page==='edges'?'同一个上游向所有关联下游发布相同的文件。':page==='receive'?'文件来自上游，选择本机保存位置后才会下载。':page==='publish'?'选择本机文件或目录，变化后自动发布给所有下游。':page==='maps'?'目录映射会自动接收上游目录中以后新增的文件。':'以最新版文件的接收回执为准，未确认不等于下载失败。'}}</p></div>
      <button v-if="page==='nodes'" class="primary" @click="name='';modal='node'"><Plus :size="17"/>创建节点</button>
      <button v-if="page==='edges'" class="primary" :disabled="nodes.length<2" @click="up='';down='';modal='edge'"><Plus :size="17"/>添加关系</button>
      <div v-if="page==='publish'" class="inline"><button :disabled="!relation.downstreams.length||busy" @click="action(()=>chooseSource(false))">选择文件</button><button class="primary" :disabled="!relation.downstreams.length||busy" @click="action(()=>chooseSource(true))"><Plus :size="17"/>选择目录</button></div>
      <button v-if="page==='receive'" :disabled="busy" @click="action(async()=>{await api('/sync','POST');notice='已通知后台同步。'})"><RefreshCw :size="17"/>立即同步</button>
    </div>
    <p v-if="error||status.state==='error'" role="alert" class="alert">{{error||status.error}}</p><p v-if="notice" role="status" class="notice">{{notice}}</p>
    <div v-if="page==='connection'" class="table-wrap"><div class="empty"><Monitor/><h3>尚未分配上下游关系</h3><p>请联系中心管理员，为这台电脑配置文件分发关系。</p><small>节点 ID：{{info.node_id}}</small><small>中心：{{info.hub}}</small></div></div>
    <div v-if="page==='nodes'" class="summary-line"><b>{{nodes.length}}</b> 个节点 <span>·</span> <b>{{edges.length}}</b> 条分发关系</div>
    <div v-if="page==='receive'||page==='deliveries'" class="toolbar"><form class="inline" @submit.prevent="offset=0;action(load)"><input v-model="search" placeholder="搜索文件路径" aria-label="搜索文件路径"/><button>搜索</button></form><select v-if="page==='receive'" v-model="filter" @change="offset=0;action(load)" aria-label="文件状态"><option value="attention">待处理文件</option><option value="">全部文件</option><option value="ok">已同步</option><option value="obsolete">上游已撤下</option><option value="conflict">路径冲突</option></select></div>
    <div class="table-wrap" v-if="page==='nodes'"><table><thead><tr><th>节点名称</th><th>职责</th><th>最近联系</th><th>节点 ID</th><th></th></tr></thead><tbody><tr v-for="n in nodes" :key="n.id"><td class="strong">{{n.name}}</td><td><span class="badge">{{n.publisher?'上游':n.subscriber?'下游':'未分配'}}</span></td><td>{{n.last_seen?new Date(n.last_seen*1000).toLocaleString():'尚未连接'}}</td><td class="mono muted">{{n.id}}</td><td><button class="text-button" @click="selected=n;name=n.name;modal='rename'">重命名</button><button class="text-button danger" @click="askRemove('removeNode',n)">删除</button></td></tr></tbody></table><div v-if="!nodes.length" class="empty"><Monitor/><h3>还没有节点</h3><p>创建第一个节点，将 ID 和令牌填入桌面客户端。</p></div></div>
    <div v-if="page==='edges'" class="table-wrap"><table><thead><tr><th>上游 · 文件来源</th><th></th><th>下游 · 文件接收方</th><th></th></tr></thead><tbody><tr v-for="e in edges" :key="e.upstream+e.downstream"><td class="strong">{{nodeName(e.upstream)}}</td><td><ArrowRight :size="18"/></td><td>{{nodeName(e.downstream)}}</td><td><button class="text-button danger" @click="askRemove('removeEdge',e)">移除关系</button></td></tr></tbody></table><div v-if="!edges.length" class="empty"><Network/><h3>还没有分发关系</h3><p>选择一个上游和一个下游，建立文件分发关系。</p></div></div>
    <div v-if="page==='publish'" class="table-wrap"><div v-if="!relation.downstreams.length" class="empty"><Upload/><h3>当前没有下游</h3><p>请先让中心管理员为此节点配置下游。</p></div><template v-else><table><thead><tr><th>发布名称</th><th>本机来源路径</th><th>设置来源</th><th></th></tr></thead><tbody><tr v-for="s in sources" :key="s.prefix"><td class="strong">{{s.prefix}}</td><td class="path">{{s.path}}</td><td>{{s.origin==='config'?'TOML 配置':'桌面设置'}}</td><td><button v-if="s.origin!=='config'" class="text-button danger" @click="askRemove('removeSource',s)">停止发布</button></td></tr></tbody></table><div v-if="!sources.length" class="empty"><Upload/><h3>选择要发布的文件</h3><p>选择一次，后续变化自动扫描并上传。</p></div></template></div>
    <div v-if="page==='maps'" class="table-wrap"><table><thead><tr><th>上游</th><th>上游路径</th><th>本机保存位置</th><th></th></tr></thead><tbody><tr v-for="m in maps" :key="m.upstream+m.from"><td>{{nodeName(m.upstream)}}</td><td>{{m.from}}</td><td class="path">{{m.to}}</td><td><span v-if="m.origin==='config'" class="muted">TOML 配置</span><button v-else class="text-button danger" @click="askRemove('removeMap',m)">取消接收</button></td></tr></tbody></table><div v-if="!maps.length" class="empty"><Settings/><h3>尚未设置保存位置</h3><p>在接收文件中认领一个文件或整个上游目录。</p></div></div>
    <div v-if="page==='receive'||page==='deliveries'" class="table-wrap"><table><thead><tr><th>文件</th><th>上游</th><th>{{page==='receive'?'本机保存位置':'下游'}}</th><th>状态</th><th v-if="page==='receive'"></th></tr></thead><tbody><tr v-for="r in rows" :key="r.upstream+r.path+(r.downstream??'')"><td><div class="strong path">{{r.path}}</div><small>{{size(r.size)}}</small></td><td>{{r.upstream_name??nodeName(r.upstream)}}</td><td class="path muted">{{page==='receive'?(r.target??'尚未指定'):r.downstream_name}}</td><td><span class="badge" :class="{good:r.received||r.state==='ok',warning:r.state==='conflict'||r.state==='error'}">{{page==='receive'?stateNames[r.state]:r.received?'已接收最新版':'未确认接收'}}</span><small v-if="r.error" class="danger">{{r.error}}</small></td><td v-if="page==='receive'"><div class="row-actions" v-if="r.state!=='obsolete'"><button @click="startClaim(r,false)">{{r.target?'更改位置':'选择位置'}}</button><button v-if="r.path.includes('/')" class="text-button" @click="startClaim(r,true)">认领目录</button></div></td></tr></tbody></table><div v-if="!rows.length" class="empty"><Files/><h3>{{search?'没有匹配的文件':page==='receive'?'当前没有此类文件':'暂无分发记录'}}</h3><p>{{page==='receive'?'上游发布后，待认领文件会出现在这里。':'建立关系并发布文件后，这里会显示接收结果。'}}</p></div></div>
    <footer v-if="page==='receive'||page==='deliveries'" class="pagination"><span>共 {{total}} 条 · 每页 50 条</span><div class="inline"><button :disabled="offset===0||busy" @click="offset=Math.max(0,offset-50);action(load)">上一页</button><span>{{Math.floor(offset/50)+1}}</span><button :disabled="offset+50>=total||busy" @click="offset+=50;action(load)">下一页</button></div></footer>
    <p v-if="desktop" class="footnote">更新覆盖前保留最近 3 份备份；上游撤下文件后，本机文件继续保留。</p>
    </main></div>
  </div>
  <div v-if="modal" class="overlay" @keydown.esc="dismiss"><section class="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title"><button class="close" aria-label="关闭" :disabled="busy" @click="dismiss"><X :size="20"/></button>
    <template v-if="modal==='node'||modal==='rename'"><h2 id="dialog-title">{{modal==='node'?'创建节点':'重命名节点'}}</h2><form @submit.prevent="modal==='node'?newNode():action(async()=>{await api('/v1/admin/nodes/'+selected!.id,'PATCH',{name:name.trim()});modal='';await load()})"><label>节点名称<input v-model="name" required maxlength="128" autofocus placeholder="例如：设计部、财务电脑"/></label><button class="primary" :disabled="busy||!name.trim()">保存</button></form></template>
    <template v-if="modal==='credential'"><h2 id="dialog-title">节点已创建</h2><p>将以下信息填入该电脑的桌面客户端。令牌只在此处显示一次，请妥善保存。</p><label>节点 ID<input readonly :value="credential?.id"/></label><label>节点令牌<textarea readonly :value="credential?.token" rows="3"></textarea></label><button class="primary" @click="saveCredential">下载节点凭证</button></template>
    <template v-if="modal==='edge'"><h2 id="dialog-title">添加分发关系</h2><form @submit.prevent="saveEdge"><label>上游 · 文件来源<select v-model="up" required><option value="" disabled>选择上游</option><option v-for="n in nodes.filter(n=>!n.subscriber)" :value="n.id" :key="n.id">{{n.name}}</option></select></label><label>下游 · 文件接收方<select v-model="down" required><option value="" disabled>选择下游</option><option v-for="n in nodes.filter(n=>!n.publisher&&n.id!==up)" :value="n.id" :key="n.id">{{n.name}}</option></select></label><p class="muted">只支持两层结构，下游不能继续向其他节点分发。</p><button class="primary" :disabled="busy||!up||!down||up===down">建立关系</button></form></template>
    <template v-if="modal==='source'"><h2 id="dialog-title">发布本机文件</h2><form @submit.prevent="saveSource"><label>本机来源路径<input v-model="local" readonly/></label><label>发布名称<input v-model="prefix" required placeholder="下游看到的文件名或目录名称"/></label><p>将发布给本节点的全部 {{relation.downstreams.length}} 个下游。目录内以后新增的文件也会发布。</p><button class="primary" :disabled="busy||!prefix.trim()">开始发布</button></form></template>
    <template v-if="modal==='claim'"><h2 id="dialog-title">{{batch?'认领上游目录':'选择本机保存位置'}}</h2><p>来源：{{nodeName(selected!.upstream)}}</p><label>上游路径<input v-model="from" readonly/></label><label>本机目录<div class="inline"><input v-model="local" readonly placeholder="请选择保存目录"/><button @click="action(chooseFolder)">浏览…</button></div></label><p>{{batch?'此目录下当前文件及以后新增的文件，都会自动接收。':'文件名沿用上游名称，设置后开始下载。'}}</p><button class="primary" :disabled="busy||!local" @click="saveClaim">保存并接收</button></template>
    <template v-if="modal.startsWith('remove')"><h2 id="dialog-title">{{modal==='removeNode'?'删除节点？':modal==='removeEdge'?'移除分发关系？':modal==='removeSource'?'停止发布？':'取消自动接收？'}}</h2><p v-if="modal==='removeNode'">将删除节点“{{selected!.name}}”及其分发关系，原令牌立即失效。电脑上已有文件会保留；重新接入需要创建新节点。</p><p v-else>此操作停止后续同步，本机和下游已有的文件会保留。正在进行的同步可能完成本轮。</p><div class="inline"><button @click="dismiss">保留</button><button class="danger-button" :disabled="busy" @click="remove">确认</button></div></template>
    <p v-if="error" class="alert" role="alert">{{error}}</p>
  </section></div>
</template>
