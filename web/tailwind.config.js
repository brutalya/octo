/** @type {import('tailwindcss').Config} */
function withOpacity(variableName) {
  return ({ opacityValue }) => {
    if (opacityValue !== undefined) {
      return `rgba(var(${variableName}), ${opacityValue})`;
    }
    return `rgb(var(${variableName}))`;
  };
}

export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  darkMode: ['class', '[data-theme="dark"]'],
  theme: {
    extend: {
      fontFamily: {
        sans: ['var(--font-sans)'],
        mono: ['var(--font-mono)'],
      },
      colors: {
        white: withOpacity('--color-white'),
        black: withOpacity('--color-black'),
        slate: {
          50: withOpacity('--color-slate-50'),
          100: withOpacity('--color-slate-100'),
          200: withOpacity('--color-slate-200'),
          300: withOpacity('--color-slate-300'),
          400: withOpacity('--color-slate-400'),
          500: withOpacity('--color-slate-500'),
          600: withOpacity('--color-slate-600'),
          700: withOpacity('--color-slate-700'),
          800: withOpacity('--color-slate-800'),
          900: withOpacity('--color-slate-900'),
        },
        indigo: {
          50: withOpacity('--color-indigo-50'),
          100: withOpacity('--color-indigo-100'),
          200: withOpacity('--color-indigo-200'),
          300: withOpacity('--color-indigo-300'),
          400: withOpacity('--color-indigo-400'),
          500: withOpacity('--color-indigo-500'),
          600: withOpacity('--color-indigo-600'),
          700: withOpacity('--color-indigo-700'),
          800: withOpacity('--color-indigo-800'),
          900: withOpacity('--color-indigo-900'),
        },
        brand: {
          50: withOpacity('--color-indigo-50'),
          100: withOpacity('--color-indigo-100'),
          200: withOpacity('--color-indigo-200'),
          300: withOpacity('--color-indigo-300'),
          400: withOpacity('--color-indigo-400'),
          500: withOpacity('--color-indigo-500'),
          600: withOpacity('--color-indigo-600'),
          700: withOpacity('--color-indigo-700'),
          800: withOpacity('--color-indigo-800'),
          900: withOpacity('--color-indigo-900'),
        },
        red: {
          50: withOpacity('--color-red-50'),
          100: withOpacity('--color-red-100'),
          200: withOpacity('--color-red-200'),
          400: withOpacity('--color-red-400'),
          500: withOpacity('--color-red-500'),
          600: withOpacity('--color-red-600'),
        },
        amber: {
          50: withOpacity('--color-amber-50'),
          100: withOpacity('--color-amber-100'),
          200: withOpacity('--color-amber-200'),
          400: withOpacity('--color-amber-400'),
          500: withOpacity('--color-amber-500'),
          600: withOpacity('--color-amber-600'),
        },
        emerald: {
          50: withOpacity('--color-emerald-50'),
          100: withOpacity('--color-emerald-100'),
          200: withOpacity('--color-emerald-200'),
          400: withOpacity('--color-emerald-400'),
          500: withOpacity('--color-emerald-500'),
          600: withOpacity('--color-emerald-600'),
        },
      },
      borderRadius: {
        'none': '0',
        'sm': 'var(--radius-sm, 0.25rem)',
        DEFAULT: 'var(--radius-md, 0.375rem)',
        'md': 'var(--radius-md, 0.375rem)',
        'lg': 'var(--radius-lg, 0.5rem)',
        'xl': 'var(--radius-xl, 0.75rem)',
        '2xl': 'var(--radius-2xl, 1rem)',
        '3xl': 'var(--radius-3xl, 1.5rem)',
        'full': '9999px',
      },
      boxShadow: {
        sm: 'var(--shadow-sm, 0 1px 2px 0 rgb(0 0 0 / 0.05))',
        DEFAULT: 'var(--shadow-md, 0 1px 3px 0 rgb(0 0 0 / 0.1))',
        md: 'var(--shadow-md, 0 4px 6px -1px rgb(0 0 0 / 0.1))',
        lg: 'var(--shadow-lg, 0 10px 15px -3px rgb(0 0 0 / 0.1))',
        xl: 'var(--shadow-xl, 0 20px 25px -5px rgb(0 0 0 / 0.1))',
        '2xl': 'var(--shadow-2xl, 0 25px 50px -12px rgb(0 0 0 / 0.25))',
      }
    },
  },
  plugins: [],
};
