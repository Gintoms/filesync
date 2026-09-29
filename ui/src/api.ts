import { invoke, isTauri } from '@tauri-apps/api/core';
export const desktop = isTauri();
let token = '';
export function setToken(value: string) { token = value; }
export async function api<T = any>(path: string, method = 'GET', body?: unknown): Promise<T> {
  if (desktop) return invoke<T>('agent_request', { path, method, body: body ?? null });
  const response = await fetch(path, {method, headers:{Authorization:`Bearer ${token}`, ...(body ? {'Content-Type':'application/json'} : {})}, body:body ? JSON.stringify(body) : undefined, cache:'no-store'});
  if (!response.ok) {
    const text = await response.text();
    let error = text;
    try { error = JSON.parse(text).error ?? text; } catch { /* plain server error */ }
    throw new Error(error || `请求失败 (${response.status})`);
  }
  return response.status === 204 ? null as T : response.json();
}
export function size(n: number) {return n < 1000 ? `${n} B` : n < 1e6 ? `${(n/1000).toFixed(1)} KB` : `${(n/1e6).toFixed(1)} MB`;}
export const stateNames: Record<string,string> = {pending:'待认领',queued:'等待下载',ok:'已同步',conflict:'路径冲突',error:'同步失败',obsolete:'上游已撤下',starting:'正在启动',syncing:'正在同步',running:'等待下次同步'};
