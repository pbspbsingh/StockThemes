CREATE TABLE IF NOT EXISTS tag_categories
(
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT     NOT NULL COLLATE NOCASE UNIQUE,
    sort_order INTEGER  NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE tags ADD COLUMN category_id INTEGER REFERENCES tag_categories (id);

CREATE INDEX IF NOT EXISTS idx_tags_category_id ON tags (category_id);

INSERT OR IGNORE INTO tag_categories (name, sort_order)
VALUES ('AI & Data', 10),
       ('Cloud, Software & Security', 20),
       ('Semiconductors & Hardware', 30),
       ('Energy & Power', 40),
       ('Materials, Agriculture & Infrastructure', 50),
       ('Mobility, Automation & Logistics', 60),
       ('Healthcare & Biotech', 70),
       ('Fintech, Crypto & Digital Consumer', 80),
       ('Space & Defense', 90),
       ('Real Assets & Travel', 100),
       ('Uncategorized', 999);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'AI & Data')
WHERE name IN (
    'AI Adopters',
    'AI Applications',
    'AI Infrastructure',
    'AI Software',
    'Data & Analytics',
    'Edge AI',
    'Quantum Computing'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Cloud, Software & Security')
WHERE name IN (
    'Cloud Computing',
    'Cloud Security',
    'Cybersecurity',
    'Developer Tools',
    'Digital Infrastructure',
    'Enterprise Software',
    'Identity Security'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Semiconductors & Hardware')
WHERE name IN (
    'Cooling & Thermal Management',
    'Data Centers',
    'Electrical Equipment',
    'Memory & Storage',
    'Networking Hardware',
    'Semiconductors'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Energy & Power')
WHERE name IN (
    'Energy Storage',
    'LNG',
    'Natural Gas',
    'Nuclear',
    'Oil & Gas',
    'Power Grid',
    'Power Infrastructure',
    'Renewable Energy',
    'Solar',
    'Uranium',
    'Utilities'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Materials, Agriculture & Infrastructure')
WHERE name IN (
    'Agriculture',
    'AgriTech',
    'Copper',
    'Critical Minerals',
    'Water Infrastructure'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Mobility, Automation & Logistics')
WHERE name IN (
    'Autonomous Vehicles',
    'Drones',
    'Electric Vehicles',
    'Industrial Automation',
    'Lab Automation',
    'Logistics & Supply Chain',
    'Robotics',
    'Supply Chain Resilience'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Healthcare & Biotech')
WHERE name IN (
    'Biotechnology',
    'Diagnostics',
    'Digital Health',
    'Drug Discovery',
    'Genomics',
    'GLP-1',
    'Medical Devices',
    'Obesity & Metabolic Health'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Fintech, Crypto & Digital Consumer')
WHERE name IN (
    'Blockchain',
    'Crypto Infrastructure',
    'Digital Advertising',
    'Digital Payments',
    'E-commerce',
    'Fintech',
    'Gaming',
    'Insurtech',
    'Social Media',
    'Streaming'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Space & Defense')
WHERE name IN (
    'Defense & Aerospace',
    'Defense Technology',
    'Launch Vehicles',
    'Satellite Communications',
    'Satellite Imagery',
    'Space Infrastructure'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Real Assets & Travel')
WHERE name IN (
    'REITs',
    'Travel & Hospitality'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Uncategorized')
WHERE category_id IS NULL;

CREATE TRIGGER IF NOT EXISTS trg_tags_default_category_after_insert
AFTER INSERT ON tags
FOR EACH ROW
WHEN NEW.category_id IS NULL
BEGIN
    UPDATE tags
    SET category_id = (SELECT id FROM tag_categories WHERE name = 'Uncategorized')
    WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_tags_default_category_after_update
AFTER UPDATE OF category_id ON tags
FOR EACH ROW
WHEN NEW.category_id IS NULL
BEGIN
    UPDATE tags
    SET category_id = (SELECT id FROM tag_categories WHERE name = 'Uncategorized')
    WHERE id = NEW.id;
END;
