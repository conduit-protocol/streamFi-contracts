/**
 * Global type declaration for CSS Modules.
 *
 * Without this, TypeScript (moduleResolution: "node", strict: true) treats
 * `import styles from './Foo.module.css'` as an error because it has no type
 * information for .css files. This declaration teaches the compiler that any
 * *.module.css import resolves to a plain object whose keys are strings
 * (the scoped class names) and whose values are strings (the mangled names
 * the bundler emits at build time).
 */
declare module '*.module.css' {
  const classes: { readonly [className: string]: string };
  export default classes;
}
