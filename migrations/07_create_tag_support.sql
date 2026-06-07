INSERT OR IGNORE INTO tags (name)
VALUES ('AI Infrastructure'),
       ('AI Software'),
       ('Aerospace'),
       ('Agriculture'),
       ('Automation'),
       ('Biotechnology'),
       ('Business Services'),
       ('Cannabis'),
       ('Capital Markets'),
       ('Clean Energy'),
       ('Cloud'),
       ('Construction'),
       ('Consumer Brands'),
       ('Critical Minerals'),
       ('Crypto'),
       ('Cybersecurity'),
       ('Data Analytics'),
       ('Data Centers'),
       ('Defense'),
       ('Developer Tools'),
       ('Diagnostics'),
       ('Digital Advertising'),
       ('Digital Health'),
       ('Digital Infrastructure'),
       ('Drones'),
       ('Education'),
       ('Electrical Equipment'),
       ('Energy Storage'),
       ('Enterprise Software'),
       ('Entertainment'),
       ('Fintech'),
       ('Future Mobility'),
       ('Gaming'),
       ('Genomics'),
       ('Hardware'),
       ('Industrial Equipment'),
       ('Insurance'),
       ('Life Science Tools'),
       ('Logistics'),
       ('Materials'),
       ('Medical Devices'),
       ('Memory & Storage'),
       ('Metabolic Health'),
       ('Networking'),
       ('Nuclear'),
       ('Oil & Gas'),
       ('Payments'),
       ('Power Grid'),
       ('Precious Metals'),
       ('Quantum'),
       ('Real Estate'),
       ('Retail'),
       ('Robotics'),
       ('Satellites'),
       ('Semiconductors'),
       ('Social Media'),
       ('Solar'),
       ('Space'),
       ('Travel'),
       ('Utilities'),
       ('Water');

INSERT OR IGNORE INTO tag_categories (name, sort_order)
VALUES ('AI & Software', 10),
       ('Compute & Data', 20),
       ('Healthcare', 30),
       ('Energy', 40),
       ('Resources', 50),
       ('Industrials', 60),
       ('Mobility & Space', 70),
       ('Finance', 80),
       ('Consumer & Media', 90),
       ('Real Estate', 100),
       ('Uncategorized', 999);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'AI & Software')
WHERE name IN (
    'AI Infrastructure',
    'AI Software',
    'Cloud',
    'Cybersecurity',
    'Data Analytics',
    'Developer Tools',
    'Enterprise Software',
    'Quantum'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Compute & Data')
WHERE name IN (
    'Data Centers',
    'Digital Infrastructure',
    'Hardware',
    'Memory & Storage',
    'Networking',
    'Semiconductors'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Healthcare')
WHERE name IN (
    'Biotechnology',
    'Cannabis',
    'Diagnostics',
    'Digital Health',
    'Genomics',
    'Life Science Tools',
    'Medical Devices',
    'Metabolic Health'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Energy')
WHERE name IN (
    'Clean Energy',
    'Energy Storage',
    'Nuclear',
    'Oil & Gas',
    'Power Grid',
    'Solar',
    'Utilities'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Resources')
WHERE name IN (
    'Agriculture',
    'Critical Minerals',
    'Materials',
    'Precious Metals',
    'Water'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Industrials')
WHERE name IN (
    'Automation',
    'Business Services',
    'Construction',
    'Defense',
    'Electrical Equipment',
    'Industrial Equipment',
    'Logistics',
    'Robotics'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Mobility & Space')
WHERE name IN (
    'Aerospace',
    'Drones',
    'Future Mobility',
    'Satellites',
    'Space'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Finance')
WHERE name IN (
    'Capital Markets',
    'Crypto',
    'Fintech',
    'Insurance',
    'Payments'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Consumer & Media')
WHERE name IN (
    'Consumer Brands',
    'Digital Advertising',
    'Education',
    'Entertainment',
    'Gaming',
    'Retail',
    'Social Media',
    'Travel'
);

UPDATE tags
SET category_id = (SELECT id FROM tag_categories WHERE name = 'Real Estate')
WHERE name IN (
    'Real Estate'
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
