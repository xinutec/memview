// @ts-check
// ESLint flat config for the Angular frontend: memview's app under src/, and
// the console's under projects/ — both are held to the same rules, and so are
// the layout harnesses and the Playwright configs that drive them. Those were
// outside every glob until 2026-08-03: eslint's own `files` did not name them,
// the lint script's paths did not reach them, `ng build` compiles only what
// `src/main.ts` imports, and Playwright strips types with esbuild rather than
// checking them. A planted `const planted: number = "not a number"` passed all
// four. The harness is the gate for how the console behaves on a phone; being
// the least-checked code in the repo was the wrong way round. Type-aware: typescript-eslint
// recommendedTypeChecked + stylisticTypeChecked (parserOptions.projectService)
// for usage bugs tsc/syntactic-lint miss (floating/misused promises, unsafe
// `any`, await-thenable), plus the Angular rules (forbid inline template:/styles:
// — the team's angular-external-template-style rule — and template a11y).

import angular from 'angular-eslint';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  {
    files: ['src/**/*.ts', 'projects/**/*.ts', 'e2e/**/*.ts', '*.config.ts'],
    extends: [
      ...tseslint.configs.recommendedTypeChecked,
      ...tseslint.configs.stylisticTypeChecked,
      ...angular.configs.tsRecommended,
    ],
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    processor: angular.processInlineTemplates,
    rules: {
      '@angular-eslint/component-max-inline-declarations': ['error', { template: 0, styles: 0 }],
      // `x as Shape` is a claim, not a check — and it is the one hole in the
      // otherwise-total protection against a value reaching the screen in the
      // wrong shape. dev-lint's DL-ANGULAR-STRINGIFIED-OBJECT types every
      // template expression honestly, so the only way to fool it is with a type
      // we manufactured ourselves. Narrow at the boundary instead.
      '@typescript-eslint/no-unsafe-type-assertion': 'error',
      '@typescript-eslint/no-empty-function': 'off',
    },
  },
  {
    // A double asserted into the interface it stands in for is the whole point
    // of a double; getting it wrong fails a test, it never reaches a user. App
    // code stays strict.
    files: ['src/**/*.spec.ts', 'projects/**/*.spec.ts'],
    rules: {
      '@typescript-eslint/no-unsafe-type-assertion': 'off',
    },
  },
  {
    files: ['src/**/*.html', 'projects/**/*.html'],
    extends: [...angular.configs.templateRecommended, ...angular.configs.templateAccessibility],
  },
);
