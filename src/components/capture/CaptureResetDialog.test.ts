import { mount, flushPromises } from '@vue/test-utils';
import { describe,it,expect,vi,beforeEach,afterEach } from 'vitest';
const api=vi.hoisted(()=>({previewCaptureReset:vi.fn(),getCaptureReset:vi.fn(),executeCaptureReset:vi.fn(),discardCaptureReset:vi.fn(async()=>{})}));
vi.mock('@/lib/capture-api',()=>({captureApi:api,pathFileName:(p:string)=>p}));
import CaptureResetDialog from './CaptureResetDialog.vue';

type Item={captureItemId:string;sourcePath:string;status:string;reason?:string|null;error:string|null};
const item=(captureItemId:string,status:string,extra:Partial<Item>={}):Item=>({captureItemId,sourcePath:`${captureItemId}.png`,status,reason:null,error:null,...extra});
const makeJob=(overrides:Record<string,unknown>={})=>({
  id:'job',status:'preview',executing:false,deleteDestinationFiles:null as boolean|null,
  allowPermanentNetworkDelete:null as boolean|null,items:[item('a','ready')],
  destinationFileCount:1,networkDestinationFileCount:0,...overrides,
});
const done=(items:Item[],overrides:Record<string,unknown>={})=>makeJob({status:'completed',executing:false,deleteDestinationFiles:true,allowPermanentNetworkDelete:false,items,...overrides});

beforeEach(()=>{vi.clearAllMocks();api.previewCaptureReset.mockResolvedValue(makeJob());});
afterEach(()=>{document.body.innerHTML='';});
function button(text:string){return [...document.body.querySelectorAll('button')].find(b=>b.textContent?.includes(text))!;}

describe('CaptureResetDialog',()=>{
 it('submits the frozen job and retained-file choice rather than rerunning the filter',async()=>{
  const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p',characterId:'c'}},attachTo:document.body});
  await flushPromises();
  const checkbox=document.body.querySelector('input')!;
  checkbox.checked=false; checkbox.dispatchEvent(new Event('change',{bubbles:true}));
  await flushPromises();
  expect(document.body.textContent).toContain('归档图和头像将保留');
  api.executeCaptureReset.mockResolvedValue(done([item('a','succeeded')],{deleteDestinationFiles:false}));
  button('确认撤销分类').click(); await flushPromises();
  expect(api.executeCaptureReset).toHaveBeenCalledWith({jobId:'job',deleteDestinationFiles:false,allowPermanentNetworkDelete:false});
  expect(api.previewCaptureReset).toHaveBeenCalledTimes(1);
  wrapper.unmount();
 });
 it('keeps the original choices when the unfinished pictures are retried',async()=>{
  api.previewCaptureReset.mockResolvedValue(done([item('a','failed',{reason:'network',error:'NAS 断线'})],{deleteDestinationFiles:false}));
  const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});await flushPromises();
  expect(document.body.querySelector('input')!.disabled).toBe(true);
  api.executeCaptureReset.mockResolvedValue(done([item('a','succeeded')],{deleteDestinationFiles:false}));
  button('重试失败项（1）').click();await flushPromises();
  expect(api.executeCaptureReset).toHaveBeenCalledWith({jobId:'job',deleteDestinationFiles:false,allowPermanentNetworkDelete:false});
  wrapper.unmount();
 });
 it('requires explicit NAS confirmation before deleting network targets',async()=>{
  api.previewCaptureReset.mockResolvedValue(makeJob({networkDestinationFileCount:1}));
  const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});await flushPromises();
  expect(button('确认撤销分类').disabled).toBe(true);
  const check=document.body.querySelectorAll('input')[1];check.checked=true;check.dispatchEvent(new Event('change',{bubbles:true}));await flushPromises();
  expect(button('确认撤销分类').disabled).toBe(false);wrapper.unmount();
 });
});

it('closes from a header button and reports progress through the styled bar',async()=>{
 const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});await flushPromises();
 const close=document.body.querySelector('button.dialog-close') as HTMLButtonElement;
 expect(close.getAttribute('aria-label')).toBe('关闭撤销分类');
 expect(document.body.querySelector('progress')).toBeNull();
 const bar=document.body.querySelector('[role="progressbar"]') as HTMLElement;
 expect(bar.getAttribute('aria-valuemax')).toBe('1');
 expect(bar.getAttribute('aria-valuenow')).toBe('0');
 expect(bar.querySelector('.reset-progress-bar')).toBeTruthy();
 close.click();await flushPromises();
 expect(wrapper.emitted('close')).toBeTruthy();
 wrapper.unmount();
});

it('shows finished items as a filled percentage',async()=>{
 api.previewCaptureReset.mockResolvedValue(done([item('a','succeeded')]));
 const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});await flushPromises();
 expect((document.body.querySelector('[role="progressbar"]') as HTMLElement).getAttribute('aria-valuenow')).toBe('1');
 expect((document.body.querySelector('.reset-progress-bar') as HTMLElement).style.width).toBe('100%');
 expect(document.body.textContent).toContain('（100%）');
 wrapper.unmount();
});

it('replaces the refresh action with a completed state once every item is done',async()=>{
 api.previewCaptureReset.mockResolvedValue(done([item('a','succeeded')]));
 const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});await flushPromises();
 expect(document.body.textContent).toContain('已全部完成');
 expect(document.body.querySelector('.reset-complete')!.textContent).toContain('任务完成');
 expect(document.body.querySelector('.reset-progress')!.getAttribute('data-state')).toBe('ok');
 expect(button('刷新进度')).toBeUndefined();
 expect(button('重试')).toBeUndefined();
 button('任务完成').click();await flushPromises();
 expect(wrapper.emitted('close')).toBeTruthy();
 wrapper.unmount();
});

it('explains unfinished items with the reason and how to fix them',async()=>{
 api.previewCaptureReset.mockResolvedValue(done([
  item('a','succeeded'),
  item('b','failed',{reason:'network',error:'The network path was not found. (os error 53)'}),
  item('c','failed',{reason:'locked',error:'The process cannot access the file. (os error 32)'}),
 ]));
 const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});await flushPromises();
 expect(document.body.textContent).toContain('已完成 1 项，2 项失败');
 const warning=document.body.querySelector('.partial-warning')!;
 expect(warning.textContent).toContain('2 项未完成');
 expect(warning.textContent).toContain('原图没有被删除');
 expect(warning.textContent).toContain('NAS/网络路径不可达');
 expect(warning.textContent).toContain('确认 NAS/共享在线');
 expect(warning.textContent).toContain('关闭可能占用该文件的程序');
 expect(warning.textContent).toContain('调试日志');
 expect(document.body.querySelector('.reset-progress')!.getAttribute('data-state')).toBe('warn');
 // A finished job with failures offers the retry instead of a completion state.
 expect(document.body.querySelector('.reset-complete')).toBeNull();
 expect(button('刷新进度')).toBeUndefined();
 expect(button('重试失败项（2）')).toBeTruthy();
 const listed=[...document.body.querySelectorAll('.reset-items li')].map(entry=>entry.textContent ?? '');
 expect(listed[0]).toContain('b.png');
 expect(listed[0]).toContain('(os error 53)');
 wrapper.unmount();
});

it('keeps a running job refreshable while it works',async()=>{
 api.previewCaptureReset.mockResolvedValue(makeJob({status:'running',executing:true,deleteDestinationFiles:true,allowPermanentNetworkDelete:false,items:[item('a','running'),item('b','succeeded')]}));
 const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});await flushPromises();
 expect(document.body.textContent).toContain('重置进行中');
 expect(button('刷新进度')).toBeTruthy();
 expect(document.body.querySelector('.reset-complete')).toBeNull();
 wrapper.unmount();
});

it('drops a preview that only arrives after the dialog was closed',async()=>{
 let release:(job:unknown)=>void=()=>{};
 api.previewCaptureReset.mockImplementation(()=>new Promise(resolve=>{release=resolve;}));
 const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});
 await flushPromises();
 wrapper.unmount();
 expect(api.discardCaptureReset).not.toHaveBeenCalled();
 // The slow answer lands after the window is gone; nobody else would drop it.
 release(makeJob());
 await flushPromises();
 expect(api.discardCaptureReset).toHaveBeenCalledWith('job');
});

it('drops an unconfirmed preview on close and keeps a started job',async()=>{
 const preview=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});
 await flushPromises();
 preview.unmount();
 await flushPromises();
 expect(api.discardCaptureReset).toHaveBeenCalledWith('job');

 api.discardCaptureReset.mockClear();
 api.previewCaptureReset.mockResolvedValue(done([item('a','succeeded')]));
 const finished=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});
 await flushPromises();
 finished.unmount();
 await flushPromises();
 expect(api.discardCaptureReset).not.toHaveBeenCalled();
});

it('summarises pictures that cannot be reset instead of claiming success',async()=>{
 api.previewCaptureReset.mockResolvedValue(done([item('a','skipped',{reason:'source_gone',error:'original source is missing'})]));
 const wrapper=mount(CaptureResetDialog,{props:{input:{projectId:'p'}},attachTo:document.body});await flushPromises();
 expect(document.body.textContent).toContain('所选图片均不可撤销（1 项已跳过）');
 expect(document.body.textContent).toContain('1 张不可处理：原图已不存在或被替换（1）');
 expect(document.body.querySelector('.reset-complete')!.textContent).toContain('任务已结束');
 wrapper.unmount();
});
