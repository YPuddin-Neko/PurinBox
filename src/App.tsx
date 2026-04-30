import { BrowserRouter, Routes, Route } from 'react-router-dom';
import Sidebar from './components/Sidebar';
import Header from './components/Header';
import HomePage from './pages/HomePage';
import ScalePage from './pages/ScalePage';
import FlipPage from './pages/FlipPage';
import FilterPage from './pages/FilterPage';
import FileKeeperPage from './pages/FileKeeperPage';
import FormatConvertPage from './pages/FormatConvertPage';
import AlphaConvertPage from './pages/AlphaConvertPage';
import BatchRenamePage from './pages/BatchRenamePage';
import TaggerPage from './pages/TaggerPage';
import LabelingPage from './pages/LabelingPage';
import CropPage from './pages/CropPage';
import ConvertPage from './pages/ConvertPage';
import AugmentPage from './pages/AugmentPage';
import OrganizePage from './pages/OrganizePage';
import SettingsPage from './pages/SettingsPage';
import './styles/global.css';
import './styles/sidebar.css';
import './styles/layout.css';
import './styles/progress.css';

export default function App() {
  return (
    <BrowserRouter>
      <Sidebar />
      <div className="main-layout">
        <Header />
        <main className="main-content">
          <Routes>
            <Route path="/" element={<HomePage />} />
            <Route path="/scale" element={<ScalePage />} />
            <Route path="/flip" element={<FlipPage />} />
            <Route path="/filter" element={<FilterPage />} />
            <Route path="/file-keeper" element={<FileKeeperPage />} />
            <Route path="/format-convert" element={<FormatConvertPage />} />
            <Route path="/alpha-convert" element={<AlphaConvertPage />} />
            <Route path="/batch-rename" element={<BatchRenamePage />} />
            <Route path="/tagger" element={<TaggerPage />} />
            <Route path="/labeling" element={<LabelingPage />} />
            <Route path="/crop" element={<CropPage />} />
            <Route path="/convert" element={<ConvertPage />} />
            <Route path="/augment" element={<AugmentPage />} />
            <Route path="/organize" element={<OrganizePage />} />
            <Route path="/settings" element={<SettingsPage />} />
          </Routes>
        </main>
      </div>
    </BrowserRouter>
  );
}
