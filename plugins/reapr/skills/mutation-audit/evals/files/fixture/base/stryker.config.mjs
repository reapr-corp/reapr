/** @type {import('@stryker-mutator/api/core').PartialStrykerOptions} */
export default {
  testRunner: 'vitest',
  mutate: ['src/**/*.ts', '!src/**/*.test.ts'],
  reporters: ['clear-text'],
  coverageAnalysis: 'perTest',
};
