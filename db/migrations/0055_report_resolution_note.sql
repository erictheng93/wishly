-- staff 結案備註；舊資料維持 NULL，原有歷史備註仍保留在 audit_logs。
ALTER TABLE content_reports ADD COLUMN resolution_note text;
