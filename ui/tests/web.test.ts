import {afterEach, expect, test, vi} from 'vitest';
import {mount,flushPromises} from '@vue/test-utils';
import App from '../src/App.vue';
const mock=vi.hoisted(()=>({api:vi.fn(),setToken:vi.fn()}));
vi.mock('../src/api',()=>({desktop:false,api:mock.api,setToken:mock.setToken,size:(n:number)=>String(n),stateNames:{}}));
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn()}));
afterEach(()=>vi.clearAllMocks());
test('admin signs in, creates a node and clears credentials on logout',async()=>{
  const list=[{id:'up',name:'发布电脑',publisher:true},{id:'down',name:'接收电脑',subscriber:true}];
  mock.api.mockImplementation(async(path:string)=>path==='/v1/admin/nodes'?list:path==='/v1/admin/edges'?[]:path==='/v1/nodes'?{id:'new-id',token:'one-time-node-token'}:null);
  const wrapper=mount(App);
  await wrapper.get('input[type=password]').setValue('test-admin-token-at-least-32-characters');
  await wrapper.get('form').trigger('submit');await flushPromises();
  expect(wrapper.text()).toContain('发布电脑');
  await wrapper.findAll('button').find(b=>b.text()==='创建节点')!.trigger('click');
  await wrapper.get('.dialog input').setValue('新节点');await wrapper.get('.dialog form').trigger('submit');await flushPromises();
  expect(mock.api).toHaveBeenCalledWith('/v1/nodes','POST',{name:'新节点'});
  expect((wrapper.get('.dialog textarea').element as HTMLTextAreaElement).value).toBe('one-time-node-token');
  await wrapper.get('.close').trigger('click');
  expect(wrapper.find('textarea').exists()).toBe(false);
  await wrapper.findAll('button').find(b=>b.text()==='退出登录')!.trigger('click');
  expect(mock.setToken).toHaveBeenLastCalledWith('');expect(wrapper.text()).toContain('登录中心');
  wrapper.unmount();
});
test('topology selector excludes subscribers as publishers',async()=>{
  mock.api.mockImplementation(async(path:string)=>path==='/v1/admin/nodes'?[{id:'a',name:'上游',publisher:true},{id:'b',name:'下游',subscriber:true},{id:'c',name:'未分配'}]:[]);
  const wrapper=mount(App);await wrapper.get('input').setValue('test-admin-token-at-least-32-characters');await wrapper.get('form').trigger('submit');await flushPromises();
  await wrapper.findAll('nav button').find(b=>b.text()==='分发关系')!.trigger('click');await flushPromises();
  await wrapper.findAll('button').find(b=>b.text()==='添加关系')!.trigger('click');
  const selects=wrapper.findAll('.dialog select');
  expect(selects[0]!.findAll('option').map(o=>o.attributes('value'))).not.toContain('b');
  expect(selects[1]!.findAll('option').map(o=>o.attributes('value'))).not.toContain('a');
  wrapper.unmount();
});

test('node deletion requires confirmation and refreshes the node list',async()=>{
  let nodes=[{id:'down',name:'接收电脑',subscriber:true}];
  mock.api.mockImplementation(async(path:string,method?:string)=>{
    if(path==='/v1/admin/nodes/down'&&method==='DELETE'){nodes=[];return null;}
    return path==='/v1/admin/nodes'?nodes:[];
  });
  const wrapper=mount(App);
  await wrapper.get('input').setValue('test-admin-token-at-least-32-characters');
  await wrapper.get('form').trigger('submit');await flushPromises();
  const deletionCalls=()=>mock.api.mock.calls.filter(([,method])=>method==='DELETE');
  await wrapper.findAll('button').find(b=>b.text()==='删除')!.trigger('click');
  expect(wrapper.get('.dialog').text()).toContain('接收电脑');
  expect(wrapper.get('.dialog').text()).toContain('原令牌立即失效');
  expect(deletionCalls()).toHaveLength(0);
  await wrapper.findAll('.dialog button').find(b=>b.text()==='保留')!.trigger('click');
  expect(wrapper.find('.dialog').exists()).toBe(false);
  expect(deletionCalls()).toHaveLength(0);
  await wrapper.findAll('button').find(b=>b.text()==='删除')!.trigger('click');
  await wrapper.findAll('.dialog button').find(b=>b.text()==='确认')!.trigger('click');
  await flushPromises();
  expect(mock.api).toHaveBeenCalledWith('/v1/admin/nodes/down','DELETE');
  expect(wrapper.findAll('tbody tr')).toHaveLength(0);
  expect(wrapper.text()).toContain('节点已删除');
  wrapper.unmount();
});
