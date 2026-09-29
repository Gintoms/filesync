import {expect,test,vi} from 'vitest';
import {mount,flushPromises} from '@vue/test-utils';
import App from '../src/App.vue';
const mock=vi.hoisted(()=>({api:vi.fn(),invoke:vi.fn(),open:vi.fn()}));
vi.mock('../src/api',()=>({desktop:true,api:mock.api,setToken:vi.fn(),size:(n:number)=>String(n),stateNames:{pending:'待认领'}}));
vi.mock('@tauri-apps/api/core',()=>({invoke:mock.invoke}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:mock.open}));
test('native folder selection creates a prefix claim without fetching content',async()=>{
 mock.invoke.mockImplementation(async(name:string)=>name==='desktop_has_config'?true:{node_id:'b'});
 mock.api.mockImplementation(async(path:string)=>path==='/status'?{state:'running'}:path==='/info'?{node_id:'b'}:path==='/relations'?{upstreams:[{id:'a',name:'设计部'}],downstreams:[]}:path.startsWith('/files?')?{total:1,items:[{upstream:'a',path:'team/report.txt',size:20,state:'pending'}]}:null);
 mock.open.mockResolvedValue('/private/tmp/received');
 const wrapper=mount(App);await flushPromises();expect(wrapper.text()).toContain('team/report.txt');
 expect(wrapper.findAll('nav button').map(b=>b.text())).toEqual(['接收文件','保存位置']);
 await wrapper.findAll('button').find(b=>b.text()==='认领目录')!.trigger('click');
 await wrapper.findAll('button').find(b=>b.text()==='浏览…')!.trigger('click');await flushPromises();
 expect(mock.open).toHaveBeenCalledWith({directory:true,multiple:false});
 await wrapper.findAll('button').find(b=>b.text()==='保存并接收')!.trigger('click');await flushPromises();
 expect(mock.api).toHaveBeenCalledWith('/claim','POST',{upstream:'a',from:'team/',to:'/private/tmp/received'});
 expect(mock.api.mock.calls.some(([path])=>String(path).includes('/content/'))).toBe(false);
 wrapper.unmount();
});

test('upstream and unassigned nodes only see menus for their current role',async()=>{
 mock.invoke.mockImplementation(async(name:string)=>name==='desktop_has_config'?true:{node_id:'a'});
 let relationships={upstreams:[] as object[],downstreams:[{id:'b',name:'接收电脑'}] as object[]};
 mock.api.mockImplementation(async(path:string)=>path==='/status'?{state:'running'}:path==='/info'?{node_id:'a'}:path==='/relations'?relationships:path==='/sources'?[]:{items:[],total:0});
 const wrapper=mount(App);await flushPromises();
 expect(wrapper.findAll('nav button').map(b=>b.text())).toEqual(['发布文件','分发结果']);
 expect(wrapper.find('h1').text()).toBe('发布文件');
 relationships={upstreams:[],downstreams:[]};
 await wrapper.get('button[aria-label="刷新"]').trigger('click');await flushPromises();
 expect(wrapper.findAll('nav button').map(b=>b.text())).toEqual(['节点状态']);
 expect(wrapper.text()).toContain('尚未分配上下游关系');
 relationships={upstreams:[{id:'b',name:'上游电脑'}],downstreams:[]};
 await wrapper.get('button[aria-label="刷新"]').trigger('click');await flushPromises();
 expect(wrapper.findAll('nav button').map(b=>b.text())).toEqual(['接收文件','保存位置']);
 expect(wrapper.find('h1').text()).toBe('接收文件');
 wrapper.unmount();
});
