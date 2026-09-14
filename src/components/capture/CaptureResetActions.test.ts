import { mount,flushPromises } from '@vue/test-utils';
import { describe,it,expect,vi } from 'vitest';
vi.mock('@/lib/capture-api',()=>({captureApi:{listCaptureResetCandidates:vi.fn(async()=>['a','b','off-page'])}}));
import CaptureResetActions from './CaptureResetActions.vue';
describe('CaptureResetActions',()=>{
 it('does not show destructive or zero-selection controls in an empty list',async()=>{
  const wrapper=mount(CaptureResetActions,{props:{filter:{projectId:'p'},selectedIds:[],hasItems:false}});await flushPromises();
  expect(wrapper.find('button').exists()).toBe(false);expect(wrapper.text()).not.toContain('撤销');wrapper.unmount();
 });
 it('enters selection from the management menu and supports select all',async()=>{
  const wrapper=mount(CaptureResetActions,{props:{filter:{projectId:'p'},selectedIds:[],hasItems:true}});await flushPromises();
 await wrapper.get('summary').trigger('click');
  // A reset job lives only while its dialog is open, so the menu offers the
  // action and nothing to recover.
  expect(wrapper.get('.reset-menu-content').text()).toBe('撤销图片');
  await wrapper.get('.reset-menu-content button').trigger('click');
  expect(wrapper.emitted('update:selecting')?.[0]).toEqual([true]);
  await wrapper.setProps({ selectableIds: ['a', 'b'], selectedIds: ['a'], selecting: true });
  expect(wrapper.text()).toContain('全选');
  expect(wrapper.text()).toContain('结束多选');
  expect(wrapper.text()).not.toContain('撤销所选');
  await wrapper.findAll('button').find((button) => button.text() === '全选筛选结果')!.trigger('click');
  await flushPromises();
  expect(wrapper.emitted('selectAll')?.[0]).toEqual([['a', 'b', 'off-page']]);
  wrapper.unmount();
 });
});

it('finishes an empty selection without opening a confirmation',async()=>{
 const wrapper=mount(CaptureResetActions,{props:{filter:{projectId:'p'},selectedIds:[],hasItems:true}});await flushPromises();
 await wrapper.setProps({selecting:true});
 const finish=wrapper.findAll('button').find(button=>button.text()==='结束多选')!;
 expect(finish.attributes('disabled')).toBeUndefined();await finish.trigger('click');
 expect(wrapper.emitted('update:selecting')?.slice(-1)[0]).toEqual([false]);wrapper.unmount();
});

it('keeps selection when a confirmation is closed before execution',async()=>{
 const wrapper=mount(CaptureResetActions,{props:{filter:{projectId:'p'},selectedIds:[],hasItems:true},global:{stubs:{CaptureResetDialog:true}}});await flushPromises();
 await wrapper.setProps({selectedIds:['a'],selecting:true});
 await wrapper.findAll('button').find(button=>button.text()==='结束多选')!.trigger('click');
 expect(wrapper.emitted('clear')).toBeUndefined();
 wrapper.getComponent({name:'CaptureResetDialog'}).vm.$emit('close');await flushPromises();
 expect(wrapper.emitted('clear')).toBeUndefined();expect(wrapper.text()).toContain('已选 1 张');wrapper.unmount();
});

async function openManagementMenu() {
 // The menu is rendered inside the page in the app, so the test keeps it in
 // the document tree too: window-level dismissal listeners only see events
 // that actually propagate out of the component.
 const wrapper=mount(CaptureResetActions,{props:{filter:{projectId:'p'},selectedIds:[],hasItems:true},attachTo:document.body});
 await flushPromises();
 const menu=wrapper.get('details').element as HTMLDetailsElement;
 menu.open=true;
 menu.dispatchEvent(new Event('toggle'));
 await flushPromises();
 return { wrapper, menu };
}

it('closes the management menu when clicking outside it',async()=>{
 const { wrapper, menu }=await openManagementMenu();
 document.body.dispatchEvent(new MouseEvent('pointerdown',{bubbles:true}));
 await flushPromises();
 expect(menu.open).toBe(false);
 wrapper.unmount();
});

it('closes the management menu when focus leaves it',async()=>{
 const { wrapper, menu }=await openManagementMenu();
 menu.dispatchEvent(new FocusEvent('focusout',{bubbles:true,relatedTarget:document.body}));
 await flushPromises();
 expect(menu.open).toBe(false);
 wrapper.unmount();
});

it('keeps the management menu open while the menu itself scrolls',async()=>{
 const { wrapper, menu }=await openManagementMenu();
 menu.dispatchEvent(new Event('scroll'));
 await flushPromises();
 expect(menu.open).toBe(true);
 wrapper.unmount();
});
