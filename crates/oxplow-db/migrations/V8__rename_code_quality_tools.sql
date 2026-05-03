-- Rename code_quality_scan.tool values from the legacy CLI names
-- (lizard / jscpd) to the analysis-kind names that survived the
-- subprocess removal.
UPDATE code_quality_scan SET tool = 'metrics' WHERE tool = 'lizard';
UPDATE code_quality_scan SET tool = 'duplication' WHERE tool = 'jscpd';
