import Navbar from "./components/Navbar";
import Hero from "./components/Hero";
import Features from "./components/Features";
import AiSection from "./components/AiSection";
import DatabaseSupport from "./components/DatabaseSupport";
import Downloads from "./components/Downloads";
import Footer from "./components/Footer";

export default function App() {
  return (
    <div className="min-h-screen bg-bg-primary text-text-primary">
      <Navbar />
      <Hero />
      <Features />
      <AiSection />
      <DatabaseSupport />
      <Downloads />
      <Footer />
    </div>
  );
}
