import { createApp, h, nextTick, type Component } from 'vue'

// Mount real Vue components; only the API and Nuxt runtime are replaced in tests.
export function mount(component: Component, props: Record<string, any> = {}, components: Record<string, Component> = {}, globals: Record<string, any> = {}) {
  const el = document.createElement('div')
  document.body.append(el)
  const app = createApp({ render: () => h(component, props) })
  for (const [name, value] of Object.entries(components)) app.component(name, value)
  Object.assign(app.config.globalProperties, globals)
  app.mount(el)
  return { el, unmount: () => { app.unmount(); el.remove() } }
}

export async function input(el: Element, selector: string, value: string) {
  const field = el.querySelector<HTMLInputElement>(selector)!
  field.value = value
  field.dispatchEvent(new Event('input', { bubbles: true }))
  await nextTick()
}

export async function submit(el: Element) {
  el.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  await nextTick()
}

export async function flush() {
  await Promise.resolve()
  await nextTick()
}
