/** @type {import('tailwindcss').Config} */
export default {
  darkMode: "class",
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        claude: {
          bg: "#0B0D11",
          card: "#131720",
          border: "#222938",
          accent: "#D97757",
          accentHover: "#E5896B",
          muted: "#8B95A8",
        }
      }
    },
  },
  plugins: [],
}
