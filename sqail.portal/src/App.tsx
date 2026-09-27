import Navbar from "./components/Navbar";
import Hero from "./components/Hero";
import Features from "./components/Features";
import Screenshots from "./components/Screenshots";
import ServiceSection from "./components/ServiceSection";
import DatabaseSupport from "./components/DatabaseSupport";
import Downloads from "./components/Downloads";
import Docs from "./components/Docs";
import Changelog from "./components/Changelog";
import Footer from "./components/Footer";

export default function App() {
  return (
    <div className="min-h-screen bg-bg-primary text-text-primary">
      <Navbar />
      <Hero />
      <Features />
      <Screenshots />
      <ServiceSection />
      <DatabaseSupport />
      <Downloads />
      <Docs />
      <Changelog />
      <Footer />
    </div>
  );
}
