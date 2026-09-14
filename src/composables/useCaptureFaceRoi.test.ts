import { defineComponent,ref,nextTick } from 'vue';
import { mount,flushPromises } from '@vue/test-utils';
import { describe,it,expect,vi } from 'vitest';
const api=vi.hoisted(()=>({getCaptureFaceRoi:vi.fn(async()=>null),setCaptureFaceRoi:vi.fn()}));
vi.mock('@/lib/capture-api',()=>({captureApi:api}));
import { useCaptureFaceRoi } from './useCaptureFaceRoi';
import type { CaptureItem } from '@/lib/capture-api';
function item(id:string):CaptureItem{return {id,manualFaceRoiJson:null,manualFaceRoiReady:0,processingVersion:0} as CaptureItem;}
describe('useCaptureFaceRoi',()=>{
 it('blocks submission while recognizing and ignores old responses after changing images',async()=>{
  const selected=ref<CaptureItem|null>(item('a'));const update=vi.fn();let state!:ReturnType<typeof useCaptureFaceRoi>;
  const wrapper=mount(defineComponent({setup(){state=useCaptureFaceRoi(selected,update);return()=>null;}}));await flushPromises();
  let finish!:(value:CaptureItem)=>void;api.setCaptureFaceRoi.mockReturnValue(new Promise(resolve=>{finish=resolve;}));
  const work=state.save({x:0,y:0,width:.5,height:.5});await nextTick();expect(state.busy.value).toBe(true);
  selected.value=item('b');await flushPromises();expect(state.busy.value).toBe(false);
  finish({...item('a'),processingVersion:1});await work;expect(update).toHaveBeenCalledTimes(1);expect(state.roi.value).toBe(null);wrapper.unmount();
 });
 it('allows abandoning a failed ROI even if automatic recognition is unavailable',async()=>{
  const selected=ref<CaptureItem|null>(item('a'));let state!:ReturnType<typeof useCaptureFaceRoi>;
  const wrapper=mount(defineComponent({setup(){state=useCaptureFaceRoi(selected,value=>{selected.value=value;});return()=>null;}}));await flushPromises();
  api.setCaptureFaceRoi.mockRejectedValue(new Error('engine unavailable'));
  await state.save(null);expect(state.blocked.value).toBe(false);wrapper.unmount();
 });
});
