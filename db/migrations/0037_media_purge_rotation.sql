-- 刪除 origin 的嘗試時間：drain 依此輪替，避免一批永久失敗的 key 一直佔住每輪 100 筆而餓死其他待辦。
ALTER TABLE user_media ADD COLUMN purge_tried_at timestamptz;
