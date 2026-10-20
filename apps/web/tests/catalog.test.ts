import { expect, it } from 'vitest'
import { catalogQuery } from '../app/utils/catalog'

it('catalogQuery: drops empty filters and page 1', () => {
  expect(catalogQuery({ brand: '', recipient: 'baby', budget: undefined })).toEqual({ recipient: 'baby' })
  expect(catalogQuery({ brand: 'x' }, 1)).toEqual({ brand: 'x' })
  expect(catalogQuery({ occasion: 'holiday' }, 3)).toEqual({ occasion: 'holiday', page: 3 })
})
