import { invoke } from '@tauri-apps/api/core';

const previewModels=typeof location!=='undefined'&&new URLSearchParams(location.search).get('preview')==='models';

async function previewApi<T>(command:string,args?:Record<string,unknown>):Promise<T>{
 if(command==='runtime_schema')return JSON.stringify({defaults:{},fields:[]}) as T;
 if(command==='mineru_schema')return JSON.stringify({defaults:{enabled:false,provider:'302',has_key:false,request_timeout_secs:60,task_timeout_secs:1800,poll_interval_secs:2,concurrency:1,model_version:'vlm',ocr:false,table:true,formula:true},keys:{},endpoints:{}}) as T;
 if(command==='list_models'){
  const url=String((args?.config as {url?:string}|undefined)?.url??'');
  const key=String(args?.key??'');
  if(url.includes('fail.example')||key.includes('bad-list'))throw new Error('模型请求失败（HTTP 503）：unavailable');
  return ['mock-alpha','mock-beta','mock-gamma'] as T;
 }
 if(command==='test_model'){
  const key=String(args?.key??'');
  if(!key||key.includes('bad'))throw new Error('模型请求失败（HTTP 401）：invalid');
  return '连接成功' as T;
 }
 return null as T;
}

export const api=<T>(command:string,args?:Record<string,unknown>)=>previewModels?previewApi<T>(command,args):invoke<T>(command,args);
