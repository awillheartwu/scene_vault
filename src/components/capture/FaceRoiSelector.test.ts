import { mount } from '@vue/test-utils';
import { describe, it, expect } from 'vitest';
import FaceRoiSelector from './FaceRoiSelector.vue';
const props = { itemId:'one', imageUrl:'blob:one', modelValue:null };
describe('FaceRoiSelector', () => {
  it('only submits after confirmation, clamps pointer coordinates to the image, and cancels on image change', async () => {
    const wrapper=mount(FaceRoiSelector,{props});
    await wrapper.get('button').trigger('click');
    const area=wrapper.get('.roi-image');
    wrapper.get('img').element.getBoundingClientRect=()=>({left:100,top:50,width:400,height:200} as DOMRect);
    await area.trigger('pointerdown',{button:0,pointerId:1,clientX:200,clientY:100});
    await area.trigger('pointermove',{pointerId:1,clientX:600,clientY:300});
    await area.trigger('pointerup',{pointerId:1,clientX:600,clientY:300});
    expect(wrapper.emitted('confirm')).toBeUndefined();
    await wrapper.findAll('button').find(b=>b.text()==='确认主脸')!.trigger('click');
    expect(wrapper.emitted('confirm')?.[0]).toEqual([{x:.25,y:.25,width:.75,height:.75}]);
    await wrapper.get('button').trigger('click');
    await wrapper.setProps({itemId:'two'});
    expect(wrapper.find('.face-roi-selector.editing').exists()).toBe(false);
    wrapper.unmount();
  });
  it('maps letterboxed portrait coordinates to the original image', async () => {
    const wrapper=mount(FaceRoiSelector,{props});
    await wrapper.get('button').trigger('click');
    const img=wrapper.get('img').element;
    Object.defineProperty(img,'naturalWidth',{value:100});
    Object.defineProperty(img,'naturalHeight',{value:200});
    img.getBoundingClientRect=()=>({left:100,top:50,width:400,height:200} as DOMRect);
    const area=wrapper.get('.roi-image');
    await area.trigger('pointerdown',{button:0,clientX:275,clientY:100});
    await area.trigger('pointerup',{clientX:325,clientY:200});
    await wrapper.findAll('button').find(b=>b.text()==='确认主脸')!.trigger('click');
    expect(wrapper.emitted('confirm')?.[0]).toEqual([{x:.25,y:.25,width:.5,height:.5}]);
    wrapper.unmount();
  });
  it('supports keyboard adjustment and Escape cancellation without recognizing', async () => {
    const wrapper=mount(FaceRoiSelector,{props});
    await wrapper.get('button').trigger('click');
    await wrapper.get('.roi-image').trigger('keydown',{key:'ArrowRight'});
    expect(wrapper.get('.roi-box').attributes('style')).toContain('left: 26%');
    await wrapper.get('.roi-image').trigger('keydown',{key:'Escape'});
    expect(wrapper.find('.face-roi-selector.editing').exists()).toBe(false);
    expect(wrapper.emitted('confirm')).toBeUndefined();
    wrapper.unmount();
  });
  it('starts framing on the detected face box and still allows a manual default', async () => {
    const detected=mount(FaceRoiSelector,{props:{...props,faceBox:{x:400,y:100,width:200,height:200}}});
    const img=detected.get('img').element;
    Object.defineProperty(img,'naturalWidth',{value:1000});
    Object.defineProperty(img,'naturalHeight',{value:1000});
    await detected.get('button').trigger('click');
    expect(detected.get('.roi-box').attributes('style')).toContain('left: 40%');
    expect(detected.get('.roi-box').attributes('style')).toContain('top: 10%');
    expect(detected.get('.roi-box').attributes('style')).toContain('width: 20%');
    detected.unmount();

    const fallback=mount(FaceRoiSelector,{props});
    await fallback.get('button').trigger('click');
    expect(fallback.get('.roi-box').attributes('style')).toContain('left: 25%');
    fallback.unmount();
  });
  it('keeps the framing hint visible next to a failure message', async () => {
    const wrapper=mount(FaceRoiSelector,{props});
    await wrapper.get('button').trigger('click');
    expect(wrapper.get('.roi-hint').text()).toContain('拖动框选一张脸');
    await wrapper.setProps({error:'选区内检测到多张脸'});
    expect(wrapper.get('.roi-hint').text()).toContain('拖动框选一张脸');
    expect(wrapper.get('.roi-message.error').text()).toContain('选区内检测到多张脸');
    wrapper.unmount();
  });
  it('reports the framing state so a host can enlarge the picture', async () => {
    const wrapper=mount(FaceRoiSelector,{props});
    await wrapper.get('button').trigger('click');
    expect(wrapper.emitted('update:editing')?.[0]).toEqual([true]);
    await wrapper.get('.roi-image').trigger('keydown',{key:'Escape'});
    expect(wrapper.emitted('update:editing')?.slice(-1)[0]).toEqual([false]);
    wrapper.unmount();
  });
});
